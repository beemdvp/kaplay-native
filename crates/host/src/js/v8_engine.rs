use std::cell::RefCell;
use std::collections::HashMap;
use std::num::NonZeroI32;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Once};
use std::time::Duration;

use kaplay_native_pak::AssetSource;
use v8::{
    Context, ContextScope, FixedArray, Global, Local, MicrotasksPolicy, Module, ModuleStatus,
    OwnedIsolate, PromiseRejectEvent, PromiseRejectMessage, PromiseResolver,
};

use super::module::{self, ModuleId};
use super::{JsArg, Watchdog};
use crate::bridge;
use crate::state::HostState;

static INIT: Once = Once::new();

macro_rules! exception_text {
    ($caught:expr) => {{
        if $caught.has_terminated() {
            "interrupted".to_string()
        } else {
            let message = $caught
                .exception()
                .map(|value| value.to_rust_string_lossy($caught))
                .unwrap_or_default();
            let stack = $caught
                .stack_trace()
                .map(|value| value.to_rust_string_lossy($caught))
                .unwrap_or_default();
            if stack.is_empty() {
                message
            } else if message.is_empty() {
                stack
            } else {
                format!("{message}\n{stack}")
            }
        }
    }};
}

thread_local! {
    static MODULES: RefCell<Option<ModuleLoader>> = const { RefCell::new(None) };
}

struct ModuleLoader {
    assets: Rc<AssetSource>,
    compiled: HashMap<String, Global<Module>>,
    origins: HashMap<NonZeroI32, ModuleId>,
    failure: Option<String>,
}

impl ModuleLoader {
    fn new(assets: Rc<AssetSource>) -> Self {
        Self {
            assets,
            compiled: HashMap::new(),
            origins: HashMap::new(),
            failure: None,
        }
    }

    fn register(&mut self, scope: &v8::PinScope<'_, '_>, id: ModuleId, module: Local<Module>) {
        self.origins.insert(module.get_identity_hash(), id.clone());
        self.compiled.insert(id.name(), Global::new(scope, module));
    }

    fn origin(&self, module: Local<Module>) -> Option<&ModuleId> {
        self.origins.get(&module.get_identity_hash())
    }

    fn load<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        id: ModuleId,
    ) -> Result<Local<'s, Module>, String> {
        if let Some(module) = self.compiled.get(&id.name()) {
            return Ok(Local::new(scope, module));
        }
        let bytes = module::read_module(&self.assets, &id)?;
        let source = String::from_utf8_lossy(&bytes);
        let module = compile_module(scope, &id, &source)?;
        self.register(scope, id, module);
        Ok(module)
    }
}

fn install_loader(assets: Rc<AssetSource>) {
    MODULES.with(|cell| *cell.borrow_mut() = Some(ModuleLoader::new(assets)));
}

fn clear_loader() {
    MODULES.with(|cell| cell.borrow_mut().take());
}

fn with_loader<T>(action: impl FnOnce(&mut ModuleLoader) -> T) -> Option<T> {
    MODULES.with(|cell| cell.borrow_mut().as_mut().map(action))
}

fn register_module(scope: &v8::PinScope<'_, '_>, id: ModuleId, module: Local<Module>) {
    with_loader(|loader| loader.register(scope, id, module));
}

fn script_origin<'s>(
    scope: &v8::PinScope<'s, '_>,
    resource_name: Local<'s, v8::String>,
) -> v8::ScriptOrigin<'s> {
    v8::ScriptOrigin::new(
        scope,
        resource_name.into(),
        0,
        0,
        false,
        -1,
        None,
        false,
        false,
        true,
        None,
    )
}

fn compile_module<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    id: &ModuleId,
    source: &str,
) -> Result<Local<'s, Module>, String> {
    let name = id.name();
    let Some(source) = v8::String::new(scope, source) else {
        return Err(format!("failed to allocate the source of module '{name}'"));
    };
    let Some(resource_name) = v8::String::new(scope, &name) else {
        return Err(format!("failed to allocate the name of module '{name}'"));
    };
    let origin = script_origin(scope, resource_name);
    v8::tc_scope!(let caught, scope);
    let mut source = v8::script_compiler::Source::new(source, Some(&origin));
    if let Some(module) = v8::script_compiler::compile_module(caught, &mut source) {
        return Ok(module);
    }
    let error = format!(
        "failed to compile module '{name}': {}",
        exception_text!(caught)
    );
    caught.reset();
    Err(error)
}

fn install_callbacks(isolate: &mut OwnedIsolate) {
    isolate.set_host_import_module_dynamically_callback(import_module);
    isolate.set_host_initialize_import_meta_object_callback(initialize_import_meta);
}

extern "C" fn initialize_import_meta(
    context: Local<Context>,
    module: Local<Module>,
    meta: Local<v8::Object>,
) {
    v8::callback_scope!(unsafe scope, context);
    let url = with_loader(|loader| loader.origin(module).cloned())
        .flatten()
        .map(|id| module::module_url(&id));
    let Some(url) = url else {
        return;
    };
    let Some(key) = v8::String::new(scope, "url") else {
        return;
    };
    let Some(value) = v8::String::new(scope, &url) else {
        return;
    };
    meta.create_data_property(scope, key.into(), value.into());
}

fn throw_error(scope: &v8::PinScope<'_, '_>, message: &str) {
    let Some(message) = v8::String::new(scope, message) else {
        return;
    };
    let error = v8::Exception::type_error(scope, message);
    scope.throw_exception(error);
}

fn resolve_module<'s>(
    context: Local<'s, Context>,
    specifier: Local<'s, v8::String>,
    _import_attributes: Local<'s, FixedArray>,
    referrer: Local<'s, Module>,
) -> Option<Local<'s, Module>> {
    v8::callback_scope!(unsafe scope, context);
    let specifier = specifier.to_rust_string_lossy(scope);
    MODULES.with(|cell| {
        let mut borrowed = cell.borrow_mut();
        let Some(loader) = borrowed.as_mut() else {
            throw_error(scope, "no module loader is installed");
            return None;
        };
        let Some(base) = loader.origin(referrer).cloned() else {
            let error = format!(
                "cannot resolve module specifier '{specifier}': the importing module is not registered"
            );
            throw_error(scope, &error);
            loader.failure = Some(error);
            return None;
        };
        let resolved = module::resolve_specifier(&base, &specifier).and_then(|id| {
            loader.load(scope, id).map_err(|error| {
                format!(
                    "cannot resolve module specifier '{specifier}' imported by '{}': {error}",
                    base.name()
                )
            })
        });
        match resolved {
            Ok(module) => Some(module),
            Err(error) => {
                throw_error(scope, &error);
                loader.failure = Some(error);
                None
            }
        }
    })
}

fn import_module<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _host_defined_options: Local<'s, v8::Data>,
    resource_name: Local<'s, v8::Value>,
    specifier: Local<'s, v8::String>,
    _import_attributes: Local<'s, FixedArray>,
) -> Option<Local<'s, v8::Promise>> {
    let base = resource_name.to_rust_string_lossy(scope);
    let specifier = specifier.to_rust_string_lossy(scope);
    let resolver = PromiseResolver::new(scope)?;
    match load_dynamic(scope, &base, &specifier) {
        Ok(value) => {
            resolver.resolve(scope, value);
        }
        Err(error) => {
            let message = v8::String::new(scope, &error)?;
            let rejection = v8::Exception::type_error(scope, message);
            resolver.reject(scope, rejection);
        }
    }
    Some(resolver.get_promise(scope))
}

fn load_dynamic<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    base: &str,
    specifier: &str,
) -> Result<Local<'s, v8::Value>, String> {
    let id = module::resolve_specifier(&ModuleId::from_name(base), specifier)?;
    let module = with_loader(|loader| loader.load(scope, id))
        .ok_or_else(|| "no module loader is installed".to_string())??;
    if module.get_status() == ModuleStatus::Uninstantiated {
        instantiate_module(scope, module)?;
    }
    let namespace = module.get_module_namespace();
    if module.get_status() == ModuleStatus::Evaluated {
        return Ok(namespace);
    }
    match evaluate_module(scope, module)? {
        Some(value) if value.is_promise() => {
            let handler = namespace_handler(scope, namespace)?;
            let promise = Local::<v8::Promise>::try_from(value)
                .map_err(|_| "module evaluation did not produce a promise".to_string())?;
            promise
                .then(scope, handler)
                .map(Local::<v8::Value>::from)
                .ok_or_else(|| "failed to chain the module evaluation promise".to_string())
        }
        _ => Ok(namespace),
    }
}

fn namespace_handler<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    namespace: Local<'s, v8::Value>,
) -> Result<Local<'s, v8::Function>, String> {
    let source = v8::String::new(scope, "namespace => () => namespace")
        .ok_or_else(|| "failed to allocate the namespace handler source".to_string())?;
    let factory = v8::Script::compile(scope, source, None)
        .ok_or_else(|| "failed to compile the namespace handler".to_string())?;
    let factory = factory
        .run(scope)
        .and_then(|value| Local::<v8::Function>::try_from(value).ok())
        .ok_or_else(|| "the namespace handler is not a function".to_string())?;
    let receiver = scope.get_current_context().global(scope);
    factory
        .call(scope, receiver.into(), &[namespace])
        .and_then(|value| Local::<v8::Function>::try_from(value).ok())
        .ok_or_else(|| "failed to build the namespace handler".to_string())
}

fn exception_value_text(scope: &v8::PinScope<'_, '_>, value: Local<v8::Value>) -> String {
    let message = value.to_rust_string_lossy(scope);
    let stack = value
        .to_object(scope)
        .and_then(|object| object.get(scope, v8::String::new(scope, "stack")?.into()))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    if stack.is_empty() || stack == message {
        message
    } else {
        stack
    }
}

fn instantiate_module<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    module: Local<'s, Module>,
) -> Result<(), String> {
    v8::tc_scope!(let caught, scope);
    if module.instantiate_module(caught, resolve_module).is_some() {
        return Ok(());
    }
    let failure = with_loader(|loader| loader.failure.take()).flatten();
    Err(failure.unwrap_or_else(|| exception_text!(caught)))
}

fn evaluate_module<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    module: Local<'s, Module>,
) -> Result<Option<Local<'s, v8::Value>>, String> {
    v8::tc_scope!(let caught, scope);
    let value = module.evaluate(caught);
    if caught.has_caught() {
        return Err(exception_text!(caught));
    }
    if module.get_status() == ModuleStatus::Errored {
        return Err(exception_value_text(caught, module.get_exception()));
    }
    Ok(value)
}

fn instantiate_and_evaluate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    module: Local<'s, Module>,
) -> Result<(), String> {
    instantiate_module(scope, module)?;
    let value = evaluate_module(scope, module)?;
    scope.perform_microtask_checkpoint();
    if module.get_status() == ModuleStatus::Errored {
        return Err(exception_value_text(scope, module.get_exception()));
    }
    if let Some(value) = value {
        if let Ok(promise) = Local::<v8::Promise>::try_from(value) {
            if promise.state() == v8::PromiseState::Rejected {
                return Err(exception_value_text(scope, promise.result(scope)));
            }
        }
    }
    Ok(())
}

pub fn version() -> String {
    v8::V8::get_version().to_string()
}

fn initialize() {
    INIT.call_once(|| {
        let mut flags = String::from("--expose-gc");
        if let Ok(extra) = std::env::var("KAPLAY_NATIVE_V8_FLAGS") {
            let extra = extra.trim();
            if !extra.is_empty() {
                flags.push(' ');
                flags.push_str(extra);
            }
        }
        v8::V8::set_flags_from_string(&flags);
        let platform = v8::new_default_platform(0, false).make_shared();
        v8::V8::initialize_platform(platform);
        v8::V8::initialize();
    });
}

unsafe extern "C" fn promise_rejected(message: PromiseRejectMessage) {
    if message.get_event() != PromiseRejectEvent::PromiseRejectWithNoHandler {
        return;
    }
    let Some(value) = message.get_value() else {
        return;
    };
    let scope = unsafe { v8::CallbackScope::new(&message) };
    let scope = std::pin::pin!(scope);
    let scope = &mut scope.init();
    log::error!("unhandled rejection: {}", value.to_rust_string_lossy(scope));
}

pub struct Engine {
    isolate: RefCell<OwnedIsolate>,
    context: Global<Context>,
    _state: Rc<HostState>,
    watchdog: Option<WatchdogThread>,
}

impl Engine {
    pub fn new(state: Rc<HostState>, shim: &str, watchdog: Arc<Watchdog>) -> Result<Self, String> {
        initialize();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(MicrotasksPolicy::Explicit);
        isolate.set_capture_stack_trace_for_uncaught_exceptions(true, 10);
        isolate.set_promise_reject_callback(promise_rejected);
        install_callbacks(&mut isolate);
        install_loader(state.assets.clone());

        let context = {
            v8::scope!(let scope, &mut isolate);
            let context = Context::new(scope, Default::default());
            let global = Global::new(&*scope, context);
            let scope = &mut ContextScope::new(scope, context);
            bridge::v8_engine::install(scope, &state, shim)?;
            global
        };

        let watchdog = if std::env::var_os("KAPLAY_NATIVE_WATCHDOG").is_some() {
            Some(WatchdogThread::start(
                isolate.thread_safe_handle(),
                watchdog,
            ))
        } else {
            None
        };

        Ok(Self {
            isolate: RefCell::new(isolate),
            context,
            _state: state,
            watchdog,
        })
    }

    pub fn eval_module(&self, name: &str, source: &str) -> Result<(), String> {
        let mut isolate = self.isolate.borrow_mut();
        isolate.cancel_terminate_execution();
        v8::scope!(let scope, &mut *isolate);
        let context = Local::new(scope, &self.context);
        let scope = &mut ContextScope::new(scope, context);
        eval_module_in(scope, name, source)
    }

    pub fn eval_preload(&self, source: &str) -> Result<(), String> {
        let mut isolate = self.isolate.borrow_mut();
        isolate.cancel_terminate_execution();
        v8::scope!(let scope, &mut *isolate);
        let context = Local::new(scope, &self.context);
        let scope = &mut ContextScope::new(scope, context);
        eval_preload_in(scope, source)
    }

    pub fn drain_jobs(&self) -> usize {
        let mut isolate = self.isolate.borrow_mut();
        isolate.cancel_terminate_execution();
        v8::scope!(let scope, &mut *isolate);
        let context = Local::new(scope, &self.context);
        let scope = &mut ContextScope::new(scope, context);
        scope.perform_microtask_checkpoint();
        0
    }

    pub fn call_global(&self, name: &str, args: &[JsArg<'_>]) -> Result<(), String> {
        let mut isolate = self.isolate.borrow_mut();
        isolate.cancel_terminate_execution();
        v8::scope!(let scope, &mut *isolate);
        let context = Local::new(scope, &self.context);
        let scope = &mut ContextScope::new(scope, context);
        call_global_in(scope, name, args)
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        if let Some(watchdog) = self.watchdog.as_mut() {
            watchdog.stop();
        }
        clear_loader();
    }
}

fn eval_module_in(
    scope: &mut v8::PinScope<'_, '_>,
    name: &str,
    source: &str,
) -> Result<(), String> {
    let id = ModuleId::from_name(name);
    let module = compile_module(scope, &id, source)?;
    register_module(scope, id, module);
    instantiate_and_evaluate(scope, module)
}

fn eval_preload_in(scope: &mut v8::PinScope<'_, '_>, source: &str) -> Result<(), String> {
    if let Some(script_source) = v8::String::new(scope, source) {
        v8::tc_scope!(let caught, scope);
        if let Some(script) = v8::Script::compile(caught, script_source, None) {
            script.run(caught);
            if caught.has_caught() {
                return Err(exception_text!(caught));
            }
            caught.perform_microtask_checkpoint();
            return Ok(());
        }
        caught.reset();
    }
    let id = ModuleId::Asset("preload".to_string());
    let module = compile_module(scope, &id, source)?;
    register_module(scope, id, module);
    instantiate_and_evaluate(scope, module)?;
    expose_module_exports(scope, module)
}

fn expose_module_exports(
    scope: &mut v8::PinScope<'_, '_>,
    module: Local<v8::Module>,
) -> Result<(), String> {
    let namespace = v8::Local::<v8::Object>::try_from(module.get_module_namespace())
        .map_err(|_| "preload module namespace is not an object".to_string())?;
    let names = namespace
        .get_own_property_names(scope, v8::GetPropertyNamesArgs::default())
        .ok_or_else(|| "failed to read preload exports".to_string())?;
    let global = scope.get_current_context().global(scope);
    for index in 0..names.length() {
        let Some(key) = names.get_index(scope, index) else {
            continue;
        };
        let Ok(name) = v8::Local::<v8::String>::try_from(key) else {
            continue;
        };
        if name.to_rust_string_lossy(scope) == "default" {
            continue;
        }
        let Some(value) = namespace.get(scope, key) else {
            continue;
        };
        let _ = global.set(scope, key, value);
    }
    Ok(())
}

fn call_global_in(
    scope: &mut v8::PinScope<'_, '_>,
    name: &str,
    args: &[JsArg<'_>],
) -> Result<(), String> {
    let context = scope.get_current_context();
    let global = context.global(scope);
    let key = v8::String::new(scope, name)
        .ok_or_else(|| format!("failed to allocate the name of {name}"))?;
    let value = global
        .get(scope, key.into())
        .ok_or_else(|| format!("{name} is not defined"))?;
    let function =
        Local::<v8::Function>::try_from(value).map_err(|_| format!("{name} is not a function"))?;
    let values: Vec<Local<v8::Value>> = args.iter().map(|arg| js_value(scope, arg)).collect();
    v8::tc_scope!(let caught, scope);
    function.call(caught, global.into(), &values);
    if caught.has_caught() {
        return Err(exception_text!(caught));
    }
    Ok(())
}

fn js_value<'s>(scope: &v8::PinScope<'s, '_>, arg: &JsArg<'_>) -> Local<'s, v8::Value> {
    match arg {
        JsArg::Uint(value) => v8::Integer::new_from_unsigned(scope, *value).into(),
        JsArg::Float(value) => v8::Number::new(scope, *value).into(),
        JsArg::Bool(value) => v8::Boolean::new(scope, *value).into(),
        JsArg::Str(value) => v8::String::new(scope, value)
            .map_or_else(|| v8::String::empty(scope).into(), |value| value.into()),
    }
}

struct WatchdogThread {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl WatchdogThread {
    fn start(isolate: v8::IsolateHandle, watchdog: Arc<Watchdog>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let thread = std::thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(10));
                if !watchdog.expired() {
                    continue;
                }
                static REPORTED: AtomicBool = AtomicBool::new(false);
                if !REPORTED.swap(true, Ordering::Relaxed) {
                    log::error!(
                        "interrupting long-running javascript during {}",
                        watchdog.phase()
                    );
                }
                isolate.terminate_execution();
            }
        });
        Self {
            stop,
            thread: Some(thread),
        }
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::rc::Rc;

    use kaplay_native_pak::AssetSource;

    use super::{clear_loader, eval_module_in, eval_preload_in, initialize, install_loader};

    fn fixture(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kap-v8-{name}-{}", std::process::id()));
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

    fn with_assets<T>(
        dir: &std::path::Path,
        test: impl FnOnce(&mut v8::PinScope<'_, '_>) -> T,
    ) -> T {
        initialize();
        install_loader(Rc::new(AssetSource::from_dir(dir)));
        let mut isolate = v8::Isolate::new(Default::default());
        super::install_callbacks(&mut isolate);
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let result = test(scope);
        clear_loader();
        result
    }

    fn with_scope<T>(test: impl FnOnce(&mut v8::PinScope<'_, '_>) -> T) -> T {
        let dir = std::env::temp_dir().join(format!("kap-v8-scope-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        with_assets(&dir, test)
    }

    fn read(scope: &mut v8::PinScope<'_, '_>, expression: &str) -> String {
        let source = v8::String::new(scope, expression).unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        script.run(scope).unwrap().to_rust_string_lossy(scope)
    }

    #[test]
    fn evaluates_scripts_and_microtasks() {
        initialize();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let source = v8::String::new(
            scope,
            "globalThis.total = 0; Promise.resolve().then(() => { globalThis.total = 41 + 1; });",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        let _ = script.run(scope);
        scope.perform_microtask_checkpoint();
        let key = v8::String::new(scope, "total").unwrap();
        let global = scope.get_current_context().global(scope);
        let value = global.get(scope, key.into()).unwrap();
        assert_eq!(value.uint32_value(scope), Some(42));
    }

    #[test]
    fn module_imports_resolve_from_the_game_files() {
        let dir = fixture(
            "imports",
            &[
                (
                    "entry.js",
                    "import { answer } from './chunk.js'; globalThis.answer = answer;",
                ),
                ("chunk.js", "export const answer = 42;"),
            ],
        );
        with_assets(&dir, |scope| {
            eval_module_in(
                scope,
                "entry.js",
                "import { answer } from './chunk.js'; globalThis.answer = answer;",
            )
            .unwrap();
            assert_eq!(read(scope, "String(globalThis.answer)"), "42");
        });
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn modules_expose_import_meta_url() {
        let dir = fixture(
            "import-meta",
            &[
                ("entry.js", ""),
                ("chunk.js", "export const url = import.meta.url;"),
            ],
        );
        with_assets(&dir, |scope| {
            eval_module_in(
                scope,
                "entry.js",
                "import { url } from './chunk.js'; globalThis.chunk = url; globalThis.entry = import.meta.url;",
            )
            .unwrap();
            assert_eq!(
                read(scope, "globalThis.chunk"),
                "kaplay-native://game/chunk.js"
            );
            assert_eq!(
                read(scope, "globalThis.entry"),
                "kaplay-native://game/entry.js"
            );
        });
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn module_imports_resolve_relative_to_file_entries() {
        let dir = fixture(
            "files",
            &[
                (
                    "game/main.js",
                    "import './lib/chunk.js'; globalThis.loaded = true;",
                ),
                ("game/lib/chunk.js", "export const value = 1;"),
            ],
        );
        with_assets(&dir, |scope| {
            let entry = dir.join("game/main.js");
            eval_module_in(
                scope,
                &entry.to_string_lossy(),
                "import './lib/chunk.js'; globalThis.loaded = true; globalThis.self = import.meta.url;",
            )
            .unwrap();
            assert_eq!(read(scope, "String(globalThis.loaded)"), "true");
            assert_eq!(
                read(scope, "globalThis.self"),
                format!("file://{}", entry.display())
            );
        });
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn modules_stay_singletons_across_imports() {
        let dir = fixture(
            "singleton",
            &[
                ("entry.js", "import './counter.js'; import './counter.js';"),
                (
                    "counter.js",
                    "globalThis.count = (globalThis.count ?? 0) + 1;",
                ),
            ],
        );
        with_assets(&dir, |scope| {
            eval_module_in(
                scope,
                "entry.js",
                "import './counter.js'; import './counter.js';",
            )
            .unwrap();
            assert_eq!(read(scope, "String(globalThis.count)"), "1");
        });
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn top_level_await_rejections_are_reported() {
        let dir = fixture("top-level-await", &[("entry.js", "")]);
        with_assets(&dir, |scope| {
            let error = eval_module_in(
                scope,
                "entry.js",
                "await Promise.reject(new Error('boom'));",
            )
            .unwrap_err();
            assert!(error.contains("boom"), "{error}");
        });
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn dynamic_imports_return_the_module_namespace() {
        let dir = fixture(
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
        );
        with_assets(&dir, |scope| {
            eval_module_in(
                scope,
                "entry.js",
                "import('./chunk.js').then((module) => { globalThis.dynamic = module.answer; globalThis.dynamicUrl = module.url; });",
            )
            .unwrap();
            scope.perform_microtask_checkpoint();
            assert_eq!(read(scope, "String(globalThis.dynamic)"), "7");
            assert_eq!(
                read(scope, "globalThis.dynamicUrl"),
                "kaplay-native://game/chunk.js"
            );
        });
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn missing_imports_report_the_specifier() {
        let dir = fixture("missing", &[("entry.js", "")]);
        with_assets(&dir, |scope| {
            let error = eval_module_in(scope, "entry.js", "import './nope.js';").unwrap_err();
            assert!(error.contains("./nope.js"), "{error}");
            assert!(error.contains("not found"), "{error}");
        });
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn bare_imports_report_guidance() {
        let dir = fixture("bare", &[("entry.js", "")]);
        with_assets(&dir, |scope| {
            let error = eval_module_in(scope, "entry.js", "import 'kaplay';").unwrap_err();
            assert!(error.contains("must be bundled"), "{error}");
        });
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn module_evaluation_errors_are_reported() {
        let dir = fixture("throwing", &[("entry.js", "")]);
        with_assets(&dir, |scope| {
            let error = eval_module_in(scope, "entry.js", "throw new Error('boom');").unwrap_err();
            assert!(error.contains("boom"), "{error}");
        });
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn classic_preloads_define_globals() {
        with_scope(|scope| {
            eval_preload_in(scope, "var fromPreload = 3;").unwrap();
            assert_eq!(read(scope, "String(globalThis.fromPreload)"), "3");
        });
    }

    #[test]
    fn module_preloads_expose_named_exports_as_globals() {
        with_scope(|scope| {
            eval_preload_in(
                scope,
                "const hidden = 4; export const answer = hidden + 38; export default 'ignored';",
            )
            .unwrap();
            assert_eq!(read(scope, "String(globalThis.answer)"), "42");
            assert_eq!(read(scope, "typeof globalThis.hidden"), "undefined");
            assert_eq!(read(scope, "typeof globalThis.default"), "undefined");
        });
    }

    #[test]
    fn preload_errors_do_not_run_the_source_twice() {
        with_scope(|scope| {
            let error = eval_preload_in(
                scope,
                "globalThis.preloadRuns = (globalThis.preloadRuns ?? 0) + 1; throw new Error('boom');",
            )
            .unwrap_err();
            assert!(error.contains("boom"), "{error}");
            assert_eq!(read(scope, "String(globalThis.preloadRuns)"), "1");
        });
    }
}
