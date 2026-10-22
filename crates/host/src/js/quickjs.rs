use std::ffi::{CStr, CString};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use kaplay_native_pak::AssetSource;
use rquickjs::function::Args;
use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::{Context, Function, Runtime};

use super::module::{self, ModuleId};
use super::{JsArg, Watchdog};
use crate::bridge;
use crate::state::HostState;

pub const MAX_PENDING_JOBS: usize = 1_000_000;

pub struct Engine {
    runtime: Runtime,
    context: Context,
}

impl Engine {
    pub fn new(state: Rc<HostState>, shim: &str, watchdog: Arc<Watchdog>) -> Result<Self, String> {
        let runtime = Runtime::new().map_err(|error| error.to_string())?;
        runtime.set_loader(AssetResolver, AssetLoader::new(state.assets.clone()));
        install_watchdog(&runtime, watchdog);
        install_rejection_tracker(&runtime);
        let context = Context::full(&runtime).map_err(|error| error.to_string())?;
        context
            .with(|ctx| bridge::quickjs::install(&ctx, state, shim))
            .map_err(|error| error.to_string())?;
        Ok(Self { runtime, context })
    }

    pub fn eval_module(&self, name: &str, source: &str) -> Result<(), String> {
        evaluate_module(&self.context, name, source)
    }

    pub fn eval_preload(&self, source: &str) -> Result<(), String> {
        self.context.with(|ctx| evaluate_preload(&ctx, source))
    }

    pub fn drain_jobs(&self) -> usize {
        self.context.with(|ctx| {
            let jobs = run_pending_jobs(&ctx);
            if jobs >= MAX_PENDING_JOBS {
                log::warn!("stopped after {jobs} pending jobs");
            }
            let pending: rquickjs::Value = ctx.catch();
            if let Some(exception) = pending.as_exception() {
                log::error!(
                    "drain exception: {} :: {}",
                    exception.message().unwrap_or_default(),
                    exception.stack().unwrap_or_default()
                );
            }
            jobs
        })
    }

    pub fn collect_garbage(&self) -> usize {
        self.runtime.run_gc();
        self.drain_jobs()
    }

    pub fn call_global(&self, name: &str, args: &[JsArg<'_>]) -> Result<(), String> {
        self.context.with(|ctx| -> Result<(), String> {
            let function: Function = ctx
                .globals()
                .get(name)
                .map_err(|error| js_error(&ctx, error))?;
            let mut call_args = Args::new(ctx.clone(), args.len());
            for arg in args {
                let pushed = match arg {
                    JsArg::Uint(value) => call_args.push_arg(*value),
                    JsArg::Float(value) => call_args.push_arg(*value),
                    JsArg::Bool(value) => call_args.push_arg(*value),
                    JsArg::Str(value) => call_args.push_arg(*value),
                };
                pushed.map_err(|error| js_error(&ctx, error))?;
            }
            call_args
                .apply::<()>(&function)
                .map_err(|error| js_error(&ctx, error))
        })
    }
}

struct AssetResolver;

impl Resolver for AssetResolver {
    fn resolve<'js>(
        &mut self,
        _ctx: &rquickjs::Ctx<'js>,
        base: &str,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        let base = ModuleId::from_name(base);
        module::resolve_specifier(&base, name)
            .map(|id| id.name())
            .map_err(|error| rquickjs::Error::new_resolving_message(base.name(), name, error))
    }
}

struct AssetLoader {
    assets: Rc<AssetSource>,
}

impl AssetLoader {
    fn new(assets: Rc<AssetSource>) -> Self {
        Self { assets }
    }
}

impl Loader for AssetLoader {
    fn load<'js>(
        &mut self,
        ctx: &rquickjs::Ctx<'js>,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<rquickjs::Module<'js, rquickjs::module::Declared>> {
        let id = ModuleId::from_name(name);
        let source = module::read_module(&self.assets, &id)
            .map_err(|error| rquickjs::Error::new_loading_message(name, error))?;
        let module = rquickjs::Module::declare(ctx.clone(), name.to_string(), source)?;
        module.meta()?.set("url", module::module_url(&id))?;
        Ok(module)
    }
}

fn install_watchdog(runtime: &Runtime, watchdog: Arc<Watchdog>) {
    if std::env::var_os("KAPLAY_NATIVE_WATCHDOG").is_none() {
        return;
    }
    runtime.set_interrupt_handler(Some(Box::new(move || {
        if watchdog.expired() {
            static REPORTED: AtomicBool = AtomicBool::new(false);
            if !REPORTED.swap(true, Ordering::Relaxed) {
                log::error!(
                    "interrupting long-running javascript during {}",
                    watchdog.phase()
                );
            }
            return true;
        }
        false
    })));
}

fn install_rejection_tracker(runtime: &Runtime) {
    runtime.set_host_promise_rejection_tracker(Some(Box::new(
        |_ctx, _promise, reason, handled| {
            if handled {
                return;
            }
            if let Some(exception) = reason.as_exception() {
                log::error!(
                    "unhandled rejection: {} :: {}",
                    exception.message().unwrap_or_default(),
                    exception.stack().unwrap_or_default()
                );
            } else {
                log::error!("unhandled rejection (non-error)");
            }
        },
    )));
}

fn evaluate_script_source(ctx: &rquickjs::Ctx<'_>, source: &str) -> Result<(), String> {
    ctx.eval::<(), _>(source.as_bytes())
        .map_err(|error| js_error(ctx, error))?;
    run_pending_jobs(ctx);
    Ok(())
}

fn evaluate_preload(ctx: &rquickjs::Ctx<'_>, source: &str) -> Result<(), String> {
    if script_compiles(ctx, source) {
        return evaluate_script_source(ctx, source);
    }
    evaluate_module_source(ctx, source)
}

fn script_compiles(ctx: &rquickjs::Ctx<'_>, source: &str) -> bool {
    let Ok(source) = CString::new(source) else {
        return false;
    };
    let flags = (rquickjs::qjs::JS_EVAL_TYPE_GLOBAL
        | rquickjs::qjs::JS_EVAL_FLAG_STRICT
        | rquickjs::qjs::JS_EVAL_FLAG_COMPILE_ONLY) as i32;
    let value = unsafe {
        rquickjs::qjs::JS_Eval(
            ctx.as_raw().as_ptr(),
            source.as_ptr(),
            source.as_bytes().len() as u64,
            c"preload".as_ptr(),
            flags,
        )
    };
    let failed =
        unsafe { rquickjs::qjs::JS_VALUE_GET_TAG(value) == rquickjs::qjs::JS_TAG_EXCEPTION };
    unsafe { rquickjs::qjs::JS_FreeValue(ctx.as_raw().as_ptr(), value) };
    if failed {
        let _ = ctx.catch();
    }
    !failed
}

fn evaluate_module_source(ctx: &rquickjs::Ctx<'_>, source: &str) -> Result<(), String> {
    let module = rquickjs::Module::declare(
        ctx.clone(),
        "preload".to_string(),
        source.as_bytes().to_vec(),
    )
    .map_err(|error| js_error(ctx, error))?;
    module
        .meta()
        .and_then(|meta| meta.set("url", module::module_url(&ModuleId::from_name("preload"))))
        .map_err(|error| js_error(ctx, error))?;
    let (module, promise) = module.eval().map_err(|error| js_error(ctx, error))?;
    run_pending_jobs(ctx);
    reject_module_evaluation(ctx, &promise)?;
    let namespace = module.namespace().map_err(|error| js_error(ctx, error))?;
    expose_namespace(ctx, &namespace)
}

fn expose_namespace<'js>(
    ctx: &rquickjs::Ctx<'js>,
    namespace: &rquickjs::Object<'js>,
) -> Result<(), String> {
    let globals = ctx.globals();
    for name in namespace.keys::<String>() {
        let name = name.map_err(|error| js_error(ctx, error))?;
        if name == "default" {
            continue;
        }
        let value: rquickjs::Value = namespace
            .get(name.as_str())
            .map_err(|error| js_error(ctx, error))?;
        globals
            .set(name.as_str(), value)
            .map_err(|error| js_error(ctx, error))?;
    }
    Ok(())
}

fn evaluate_module(context: &Context, name: &str, source: &str) -> Result<(), String> {
    context.with(|ctx| {
        let module =
            rquickjs::Module::declare(ctx.clone(), name.to_string(), source.as_bytes().to_vec())
                .map_err(|error| js_error(&ctx, error))?;
        module
            .meta()
            .and_then(|meta| meta.set("url", module::module_url(&ModuleId::from_name(name))))
            .map_err(|error| js_error(&ctx, error))?;
        let (_, promise) = module.eval().map_err(|error| js_error(&ctx, error))?;
        run_pending_jobs(&ctx);
        reject_module_evaluation(&ctx, &promise)
    })
}

fn reject_module_evaluation(
    ctx: &rquickjs::Ctx<'_>,
    promise: &rquickjs::Promise<'_>,
) -> Result<(), String> {
    match promise.result::<rquickjs::Value>() {
        Some(Err(error)) => Err(js_error(ctx, error)),
        _ => Ok(()),
    }
}

fn js_error(ctx: &rquickjs::Ctx<'_>, error: rquickjs::Error) -> String {
    let pending: rquickjs::Value = ctx.catch();
    if let Some(exception) = pending.as_exception() {
        let message = exception.message().unwrap_or_default();
        let stack = exception.stack().unwrap_or_default();
        if stack.is_empty() {
            return message;
        }
        return format!("{message}\n{stack}");
    }
    error.to_string()
}

pub fn run_pending_jobs(ctx: &rquickjs::Ctx<'_>) -> usize {
    let mut jobs = 0;
    while jobs < MAX_PENDING_JOBS {
        if !run_one_job(ctx) {
            break;
        }
        jobs += 1;
    }
    jobs
}

fn run_one_job(ctx: &rquickjs::Ctx<'_>) -> bool {
    let raw = ctx.as_raw().as_ptr();
    unsafe {
        let runtime = rquickjs::qjs::JS_GetRuntime(raw);
        let mut job_ctx: *mut rquickjs::qjs::JSContext = std::ptr::null_mut();
        let result = rquickjs::qjs::JS_ExecutePendingJob(runtime, &mut job_ctx);
        if result < 0 && !job_ctx.is_null() {
            log_exception(job_ctx);
        }
        result != 0
    }
}

unsafe fn log_exception(ctx: *mut rquickjs::qjs::JSContext) {
    let exception = rquickjs::qjs::JS_GetException(ctx);
    let stack = rquickjs::qjs::JS_GetPropertyStr(ctx, exception, c"stack".as_ptr());
    let text = rquickjs::qjs::JS_ToCString(ctx, stack);
    if text.is_null() {
        let message = rquickjs::qjs::JS_GetPropertyStr(ctx, exception, c"message".as_ptr());
        let pointer = rquickjs::qjs::JS_ToCString(ctx, message);
        if !pointer.is_null() {
            let value = CStr::from_ptr(pointer).to_string_lossy();
            log::error!("uncaught error in job: {value}");
            rquickjs::qjs::JS_FreeCString(ctx, pointer);
        }
        rquickjs::qjs::JS_FreeValue(ctx, message);
    } else {
        let value = CStr::from_ptr(text).to_string_lossy();
        log::error!("uncaught error in job: {value}");
        rquickjs::qjs::JS_FreeCString(ctx, text);
    }
    rquickjs::qjs::JS_FreeValue(ctx, stack);
    rquickjs::qjs::JS_FreeValue(ctx, exception);
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::rc::Rc;

    use kaplay_native_pak::AssetSource;
    use rquickjs::{Context, Runtime};

    use super::{
        evaluate_module, evaluate_preload, evaluate_script_source, run_pending_jobs, AssetLoader,
        AssetResolver,
    };

    fn fixture(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kap-quickjs-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        for (path, source) in files {
            let full = dir.join(path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&full, source).unwrap();
        }
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn with_fixture<T>(name: &str, files: &[(&str, &str)], test: impl FnOnce(&Context) -> T) -> T {
        let dir = fixture(name, files);
        let runtime = Runtime::new().unwrap();
        runtime.set_loader(
            AssetResolver,
            AssetLoader::new(Rc::new(AssetSource::from_dir(&dir))),
        );
        let context = Context::full(&runtime).unwrap();
        let result = test(&context);
        std::fs::remove_dir_all(dir).ok();
        result
    }

    #[test]
    fn module_imports_resolve_from_the_game_files() {
        with_fixture(
            "imports",
            &[
                (
                    "entry.js",
                    "import { answer } from './chunk.js'; globalThis.answer = answer;",
                ),
                ("chunk.js", "export const answer = 42;"),
            ],
            |context| {
                evaluate_module(
                    context,
                    "entry.js",
                    "import { answer } from './chunk.js'; globalThis.answer = answer;",
                )
                .unwrap();
                context.with(|ctx| {
                    let answer: i32 = ctx.eval("globalThis.answer").unwrap();
                    assert_eq!(answer, 42);
                });
            },
        );
    }

    #[test]
    fn modules_expose_import_meta_url() {
        with_fixture(
            "import-meta",
            &[
                ("entry.js", ""),
                ("chunk.js", "export const url = import.meta.url;"),
            ],
            |context| {
                evaluate_module(
                    context,
                    "entry.js",
                    "import { url } from './chunk.js'; globalThis.chunk = url; globalThis.entry = import.meta.url;",
                )
                .unwrap();
                context.with(|ctx| {
                    let chunk: String = ctx.eval("globalThis.chunk").unwrap();
                    assert_eq!(chunk, "kaplay-native://game/chunk.js");
                    let entry: String = ctx.eval("globalThis.entry").unwrap();
                    assert_eq!(entry, "kaplay-native://game/entry.js");
                });
            },
        );
    }

    #[test]
    fn dynamic_imports_return_the_module_namespace() {
        with_fixture(
            "dynamic",
            &[
                (
                    "entry.js",
                    "import('./chunk.js').then((module) => { globalThis.dynamic = module.answer; globalThis.dynamicUrl = module.url; });",
                ),
                (
                    "chunk.js",
                    "export const answer = 7; export const url = import.meta.url;",
                ),
            ],
            |context| {
                evaluate_module(
                    context,
                    "entry.js",
                    "import('./chunk.js').then((module) => { globalThis.dynamic = module.answer; globalThis.dynamicUrl = module.url; });",
                )
                .unwrap();
                context.with(|ctx| {
                    let dynamic: i32 = ctx.eval("globalThis.dynamic").unwrap();
                    assert_eq!(dynamic, 7);
                    let url: String = ctx.eval("globalThis.dynamicUrl").unwrap();
                    assert_eq!(url, "kaplay-native://game/chunk.js");
                });
            },
        );
    }

    #[test]
    fn missing_imports_report_the_specifier() {
        with_fixture("missing", &[("entry.js", "")], |context| {
            let error = evaluate_module(context, "entry.js", "import './nope.js';").unwrap_err();
            assert!(error.contains("nope.js"), "{error}");
            assert!(error.contains("not found"), "{error}");
        });
    }

    #[test]
    fn file_entries_resolve_their_own_imports() {
        let dir = fixture(
            "file-entry",
            &[
                ("game/main.js", ""),
                ("game/lib/chunk.js", "export const value = 3;"),
            ],
        );
        let runtime = Runtime::new().unwrap();
        runtime.set_loader(
            AssetResolver,
            AssetLoader::new(Rc::new(AssetSource::from_dir(&dir))),
        );
        let context = Context::full(&runtime).unwrap();
        let entry = dir.join("game/main.js");
        evaluate_module(
            &context,
            &entry.to_string_lossy(),
            "import { value } from './lib/chunk.js'; globalThis.file = value; globalThis.self = import.meta.url;",
        )
        .unwrap();
        context.with(|ctx| {
            let value: i32 = ctx.eval("globalThis.file").unwrap();
            assert_eq!(value, 3);
            let url: String = ctx.eval("globalThis.self").unwrap();
            assert_eq!(url, format!("file://{}", entry.display()));
        });
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn top_level_await_rejections_are_reported() {
        with_fixture("top-level-await", &[("entry.js", "")], |context| {
            let error = evaluate_module(
                context,
                "entry.js",
                "await Promise.reject(new Error('boom'));",
            )
            .unwrap_err();
            assert!(error.contains("boom"), "{error}");
        });
    }

    #[test]
    fn module_evaluation_errors_are_reported() {
        with_fixture("throwing", &[("entry.js", "")], |context| {
            let error =
                evaluate_module(context, "entry.js", "throw new Error('boom');").unwrap_err();
            assert!(error.contains("boom"), "{error}");
        });
    }

    #[test]
    fn logs_and_continues_after_a_failed_job() {
        let runtime = Runtime::new().unwrap();
        let context = Context::full(&runtime).unwrap();
        context.with(|ctx| {
            ctx.eval::<(), _>(
                b"queueMicrotask(() => { throw new Error('boom'); }); globalThis.done = false; queueMicrotask(() => { globalThis.done = true; });",
            )
            .unwrap();
            let jobs = run_pending_jobs(&ctx);
            assert_eq!(jobs, 2);
            let done: bool = ctx.eval("globalThis.done").unwrap();
            assert!(done);
            assert!(!ctx.has_exception());
        });
    }

    #[test]
    fn reports_no_jobs_when_queue_is_empty() {
        let runtime = Runtime::new().unwrap();
        let context = Context::full(&runtime).unwrap();
        context.with(|ctx| {
            assert_eq!(run_pending_jobs(&ctx), 0);
        });
    }

    #[test]
    fn module_declarations_do_not_clobber_globals() {
        let runtime = Runtime::new().unwrap();
        let context = Context::full(&runtime).unwrap();
        context
            .with(|ctx| evaluate_script_source(&ctx, "globalThis.UP = 'global';"))
            .unwrap();
        evaluate_module(
            &context,
            "game",
            "function UP() { return 7; } globalThis.result = UP(); var leaked = 1;",
        )
        .unwrap();
        context.with(|ctx| {
            let kept: String = ctx.eval("globalThis.UP").unwrap();
            let result: i32 = ctx.eval("globalThis.result").unwrap();
            let leaked: String = ctx.eval("typeof globalThis.leaked").unwrap();
            assert_eq!(kept, "global");
            assert_eq!(result, 7);
            assert_eq!(leaked, "undefined");
        });
    }

    #[test]
    fn preload_scripts_still_define_globals() {
        let runtime = Runtime::new().unwrap();
        let context = Context::full(&runtime).unwrap();
        context.with(|ctx| {
            evaluate_preload(&ctx, "var fromPreload = 3; globalThis.fromPreload = 4;").unwrap();
            let value: i32 = ctx.eval("globalThis.fromPreload").unwrap();
            assert_eq!(value, 4);
        });
    }

    #[test]
    fn module_preloads_expose_named_exports_as_globals() {
        let runtime = Runtime::new().unwrap();
        let context = Context::full(&runtime).unwrap();
        context.with(|ctx| {
            evaluate_preload(
                &ctx,
                "const hidden = 4; export const answer = hidden + 38; export default 'ignored';",
            )
            .unwrap();
            let answer: i32 = ctx.eval("globalThis.answer").unwrap();
            assert_eq!(answer, 42);
            let hidden: String = ctx.eval("typeof globalThis.hidden").unwrap();
            assert_eq!(hidden, "undefined");
            let default_export: String = ctx.eval("typeof globalThis.default").unwrap();
            assert_eq!(default_export, "undefined");
        });
    }

    #[test]
    fn module_preloads_fail_through_the_module_path() {
        let runtime = Runtime::new().unwrap();
        let context = Context::full(&runtime).unwrap();
        context.with(|ctx| {
            let error = evaluate_preload(
                &ctx,
                "import { missing } from './missing.js'; export const value = missing;",
            )
            .unwrap_err();
            assert!(error.contains("missing.js"), "{error}");
        });
    }

    #[test]
    fn preload_errors_do_not_run_the_source_twice() {
        let runtime = Runtime::new().unwrap();
        let context = Context::full(&runtime).unwrap();
        context.with(|ctx| {
            let error = evaluate_preload(
                &ctx,
                "globalThis.preloadRuns = (globalThis.preloadRuns ?? 0) + 1; throw new Error('boom');",
            )
            .unwrap_err();
            assert!(error.contains("boom"), "{error}");
            let runs: i32 = ctx.eval("globalThis.preloadRuns").unwrap();
            assert_eq!(runs, 1);
        });
    }
}
