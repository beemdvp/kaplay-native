use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::state::HostState;

pub mod module;

#[cfg(feature = "quickjs")]
mod quickjs;
#[cfg(feature = "v8")]
mod v8_engine;

#[cfg(not(any(feature = "quickjs", feature = "v8")))]
compile_error!("enable at least one of the `quickjs` or `v8` features");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsEngine {
    QuickJs,
    V8,
}

impl JsEngine {
    pub fn name(self) -> &'static str {
        match self {
            JsEngine::QuickJs => "quickjs",
            JsEngine::V8 => "v8",
        }
    }
}

impl std::fmt::Display for JsEngine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.name())
    }
}

impl std::str::FromStr for JsEngine {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "quickjs" => Ok(JsEngine::QuickJs),
            "v8" => Ok(JsEngine::V8),
            other => Err(format!(
                "unknown javascript engine '{other}' (available: {})",
                engine_list()
            )),
        }
    }
}

pub fn available_engines() -> &'static [JsEngine] {
    #[cfg(all(feature = "quickjs", feature = "v8"))]
    {
        &[JsEngine::QuickJs, JsEngine::V8]
    }
    #[cfg(all(feature = "quickjs", not(feature = "v8")))]
    {
        &[JsEngine::QuickJs]
    }
    #[cfg(all(feature = "v8", not(feature = "quickjs")))]
    {
        &[JsEngine::V8]
    }
    #[cfg(not(any(feature = "quickjs", feature = "v8")))]
    {
        &[]
    }
}

pub fn default_engine() -> JsEngine {
    #[cfg(feature = "quickjs")]
    {
        JsEngine::QuickJs
    }
    #[cfg(not(feature = "quickjs"))]
    {
        JsEngine::V8
    }
}

pub fn engine_list() -> String {
    available_engines()
        .iter()
        .map(|engine| engine.name())
        .collect::<Vec<_>>()
        .join(", ")
}

pub enum JsArg<'a> {
    Uint(u32),
    Float(f64),
    Bool(bool),
    Str(&'a str),
}

const NO_DEADLINE: u64 = u64::MAX;

pub struct Watchdog {
    start: Instant,
    deadline_ms: AtomicU64,
    phase: Mutex<&'static str>,
}

impl Watchdog {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            start: Instant::now(),
            deadline_ms: AtomicU64::new(NO_DEADLINE),
            phase: Mutex::new(""),
        })
    }

    pub fn mark(&self, phase: &'static str) {
        *self.phase.lock().unwrap() = phase;
        let deadline = self.start.elapsed().as_millis() as u64 + 4_000;
        self.deadline_ms.store(deadline, Ordering::Relaxed);
    }

    pub fn unmark(&self) {
        self.deadline_ms.store(NO_DEADLINE, Ordering::Relaxed);
    }

    pub fn expired(&self) -> bool {
        let deadline = self.deadline_ms.load(Ordering::Relaxed);
        deadline != NO_DEADLINE && self.start.elapsed().as_millis() as u64 > deadline
    }

    pub fn phase(&self) -> &'static str {
        *self.phase.lock().unwrap()
    }
}

enum Backend {
    #[cfg(feature = "quickjs")]
    QuickJs(quickjs::Engine),
    #[cfg(feature = "v8")]
    V8(v8_engine::Engine),
}

pub struct JsRuntime {
    backend: Backend,
}

impl JsRuntime {
    pub fn new(
        engine: JsEngine,
        state: Rc<HostState>,
        shim: &str,
        watchdog: Arc<Watchdog>,
    ) -> Result<Self, String> {
        let backend = match engine {
            #[cfg(feature = "quickjs")]
            JsEngine::QuickJs => Backend::QuickJs(quickjs::Engine::new(state, shim, watchdog)?),
            #[cfg(feature = "v8")]
            JsEngine::V8 => Backend::V8(v8_engine::Engine::new(state, shim, watchdog)?),
            #[allow(unreachable_patterns)]
            other => {
                return Err(format!(
                    "javascript engine '{other}' is not available in this build (available: {})",
                    engine_list()
                ))
            }
        };
        Ok(Self { backend })
    }

    pub fn name(&self) -> &'static str {
        match &self.backend {
            #[cfg(feature = "quickjs")]
            Backend::QuickJs(_) => "quickjs",
            #[cfg(feature = "v8")]
            Backend::V8(_) => "v8",
        }
    }

    pub fn version(&self) -> Option<String> {
        match &self.backend {
            #[cfg(feature = "quickjs")]
            Backend::QuickJs(_) => None,
            #[cfg(feature = "v8")]
            Backend::V8(_) => Some(v8_engine::version()),
        }
    }

    pub fn eval_preload(&self, source: &str) -> Result<(), String> {
        match &self.backend {
            #[cfg(feature = "quickjs")]
            Backend::QuickJs(engine) => engine.eval_preload(source),
            #[cfg(feature = "v8")]
            Backend::V8(engine) => engine.eval_preload(source),
        }
    }

    pub fn eval_module(&self, name: &str, source: &str) -> Result<(), String> {
        match &self.backend {
            #[cfg(feature = "quickjs")]
            Backend::QuickJs(engine) => engine.eval_module(name, source),
            #[cfg(feature = "v8")]
            Backend::V8(engine) => engine.eval_module(name, source),
        }
    }

    pub fn drain_jobs(&self) -> usize {
        match &self.backend {
            #[cfg(feature = "quickjs")]
            Backend::QuickJs(engine) => engine.drain_jobs(),
            #[cfg(feature = "v8")]
            Backend::V8(engine) => engine.drain_jobs(),
        }
    }

    pub fn call_global(&self, name: &str, args: &[JsArg<'_>]) -> Result<(), String> {
        match &self.backend {
            #[cfg(feature = "quickjs")]
            Backend::QuickJs(engine) => engine.call_global(name, args),
            #[cfg(feature = "v8")]
            Backend::V8(engine) => engine.call_global(name, args),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{available_engines, default_engine, engine_list, JsEngine};

    #[test]
    fn parses_known_engine_names() {
        assert_eq!("quickjs".parse::<JsEngine>().unwrap(), JsEngine::QuickJs);
        assert_eq!("V8".parse::<JsEngine>().unwrap(), JsEngine::V8);
        assert!("mozjs".parse::<JsEngine>().is_err());
    }

    #[test]
    fn lists_compiled_engines() {
        assert!(!available_engines().is_empty());
        assert!(available_engines().contains(&default_engine()));
        assert!(engine_list().contains(default_engine().name()));
    }
}
