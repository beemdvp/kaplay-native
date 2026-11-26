//! Dynamic Steamworks integration.
//!
//! The Steamworks redistributables are never bundled with the runtime. The
//! library is loaded at runtime from next to the executable (or from the
//! path in `KAPLAY_NATIVE_STEAM_LIB`) and the flat API symbols are resolved
//! individually, so a missing Steam client is a soft failure.

use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};

use libloading::Library;

pub const DEFAULT_APP_ID: u32 = 480;
pub const STEAM_LIB_ENV: &str = "KAPLAY_NATIVE_STEAM_LIB";

#[derive(Debug, thiserror::Error)]
pub enum SteamError {
    #[error("steam_api was not found next to the executable or in the library path")]
    LibraryMissing,
    #[error("steam_api is missing the symbol {0}")]
    SymbolMissing(String),
    #[error("SteamAPI_Init failed: {0}")]
    Init(String),
}

type RestartFn = unsafe extern "C" fn(u32) -> bool;
type InitFn = unsafe extern "C" fn() -> bool;
type InitFlatFn = unsafe extern "C" fn(*mut c_char) -> i32;
type ShutdownFn = unsafe extern "C" fn();
type RunCallbacksFn = unsafe extern "C" fn();
type IsRunningFn = unsafe extern "C" fn() -> bool;
type GetUserFn = unsafe extern "C" fn() -> *mut c_void;
type GetPlayerNameFn = unsafe extern "C" fn(*mut c_void) -> *const c_char;
type GetFriendsFn = unsafe extern "C" fn() -> *mut c_void;
type GetFriendPersonaNameFn = unsafe extern "C" fn(*mut c_void, u64) -> *const c_char;
type GetSteamIdFn = unsafe extern "C" fn(*mut c_void) -> u64;
type GetStatsFn = unsafe extern "C" fn() -> *mut c_void;
type SetAchievementFn = unsafe extern "C" fn(*mut c_void, *const c_char) -> bool;
type GetAchievementFn = unsafe extern "C" fn(*mut c_void, *const c_char, *mut bool) -> bool;
type StoreStatsFn = unsafe extern "C" fn(*mut c_void) -> bool;
type SetStatIntFn = unsafe extern "C" fn(*mut c_void, *const c_char, i32) -> bool;
type GetStatIntFn = unsafe extern "C" fn(*mut c_void, *const c_char, *mut i32) -> bool;
type SetStatFloatFn = unsafe extern "C" fn(*mut c_void, *const c_char, f32) -> bool;
type GetStatFloatFn = unsafe extern "C" fn(*mut c_void, *const c_char, *mut f32) -> bool;
type GetStorageFn = unsafe extern "C" fn() -> *mut c_void;
type FileWriteFn = unsafe extern "C" fn(*mut c_void, *const c_char, *const u8, i32) -> bool;
type FileReadFn = unsafe extern "C" fn(*mut c_void, *const c_char, *mut u8, i32) -> i32;
type GetFileSizeFn = unsafe extern "C" fn(*mut c_void, *const c_char) -> i32;
type FileExistsFn = unsafe extern "C" fn(*mut c_void, *const c_char) -> bool;
type FileDeleteFn = unsafe extern "C" fn(*mut c_void, *const c_char) -> bool;

enum InitSource {
    Flat(InitFlatFn),
    Classic(InitFn),
}

impl InitSource {
    unsafe fn call(&self) -> Result<(), SteamError> {
        match self {
            InitSource::Flat(init) => {
                let mut message = [0 as c_char; 1024];
                match (init)(message.as_mut_ptr()) {
                    0 => Ok(()),
                    code => {
                        let text = CStr::from_ptr(message.as_ptr()).to_string_lossy();
                        if text.is_empty() {
                            Err(SteamError::Init(format!("SDK result code {code}")))
                        } else {
                            Err(SteamError::Init(text.into_owned()))
                        }
                    }
                }
            }
            InitSource::Classic(init) => {
                if (init)() {
                    Ok(())
                } else {
                    Err(SteamError::Init(String::from(
                        "Steam is not running or the app id is unknown",
                    )))
                }
            }
        }
    }
}

enum NameSource {
    User(GetPlayerNameFn),
    Friends {
        friends: GetFriendsFn,
        persona: GetFriendPersonaNameFn,
    },
}

fn c_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    Some(
        unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned(),
    )
}

pub fn library_names() -> &'static [&'static str] {
    if cfg!(target_os = "windows") {
        &["steam_api64.dll", "steam_api.dll"]
    } else if cfg!(target_os = "macos") {
        &["libsteam_api.dylib"]
    } else {
        &["libsteam_api.so"]
    }
}

pub fn candidate_paths() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(explicit) = std::env::var_os(STEAM_LIB_ENV) {
        candidates.push(PathBuf::from(explicit));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for name in library_names() {
                candidates.push(dir.join(name));
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        for name in library_names() {
            candidates.push(cwd.join(name));
        }
    }
    candidates
}

fn path_looks_valid(path: &Path) -> bool {
    path.is_file()
}

pub struct SteamApi {
    _library: Library,
    restart: RestartFn,
    init: InitSource,
    shutdown: ShutdownFn,
    run_callbacks: RunCallbacksFn,
    is_running: IsRunningFn,
    get_user: GetUserFn,
    name_source: NameSource,
    get_steam_id: GetSteamIdFn,
    get_stats: GetStatsFn,
    set_achievement: SetAchievementFn,
    get_achievement: GetAchievementFn,
    store_stats: StoreStatsFn,
    set_stat_int: SetStatIntFn,
    get_stat_int: GetStatIntFn,
    set_stat_float: SetStatFloatFn,
    get_stat_float: GetStatFloatFn,
    get_storage: GetStorageFn,
    file_write: FileWriteFn,
    file_read: FileReadFn,
    file_size: GetFileSizeFn,
    file_exists: FileExistsFn,
    file_delete: FileDeleteFn,
    initialized: bool,
}

impl SteamApi {
    pub fn load() -> Option<SteamApi> {
        match Self::try_load() {
            Ok(api) => Some(api),
            Err(error) => {
                log::debug!("steam: {error}");
                None
            }
        }
    }

    pub fn try_load() -> Result<SteamApi, SteamError> {
        let path = candidate_paths()
            .into_iter()
            .find(|path| path_looks_valid(path))
            .ok_or(SteamError::LibraryMissing)?;
        Self::from_library(&path)
    }

    pub fn from_library(path: &Path) -> Result<SteamApi, SteamError> {
        unsafe {
            let library = Library::new(path).map_err(|_| SteamError::LibraryMissing)?;
            macro_rules! missing {
                ($name:literal) => {
                    SteamError::SymbolMissing(
                        String::from_utf8_lossy($name)
                            .trim_end_matches('\0')
                            .to_string(),
                    )
                };
            }
            macro_rules! symbol {
                ($name:literal) => {
                    *library
                        .get::<*const c_void>($name)
                        .map_err(|_| missing!($name))?
                };
            }
            macro_rules! optional {
                ($name:literal) => {
                    library
                        .get::<*const c_void>($name)
                        .ok()
                        .map(|symbol| *symbol)
                };
            }
            let restart: RestartFn =
                std::mem::transmute(symbol!(b"SteamAPI_RestartAppIfNecessary\0"));
            let init = match optional!(b"SteamAPI_InitFlat\0") {
                Some(flat) => {
                    InitSource::Flat(std::mem::transmute::<*const c_void, InitFlatFn>(flat))
                }
                None => InitSource::Classic(std::mem::transmute::<*const c_void, InitFn>(symbol!(
                    b"SteamAPI_Init\0"
                ))),
            };
            let shutdown: ShutdownFn = std::mem::transmute(symbol!(b"SteamAPI_Shutdown\0"));
            let run_callbacks: RunCallbacksFn =
                std::mem::transmute(symbol!(b"SteamAPI_RunCallbacks\0"));
            let is_running: IsRunningFn =
                std::mem::transmute(symbol!(b"SteamAPI_IsSteamRunning\0"));
            let get_user: GetUserFn = std::mem::transmute(symbol!(b"SteamAPI_SteamUser_v023\0"));
            let name_source = match optional!(b"SteamAPI_ISteamUser_GetPlayerName\0") {
                Some(get) => {
                    NameSource::User(std::mem::transmute::<*const c_void, GetPlayerNameFn>(get))
                }
                None => {
                    let friends = optional!(b"SteamAPI_SteamFriends_v018\0")
                        .ok_or_else(|| missing!(b"SteamAPI_SteamFriends_v018\0"))?;
                    let persona = optional!(b"SteamAPI_ISteamFriends_GetFriendPersonaName\0")
                        .ok_or_else(|| {
                            missing!(b"SteamAPI_ISteamFriends_GetFriendPersonaName\0")
                        })?;
                    NameSource::Friends {
                        friends: std::mem::transmute::<*const c_void, GetFriendsFn>(friends),
                        persona: std::mem::transmute::<*const c_void, GetFriendPersonaNameFn>(
                            persona,
                        ),
                    }
                }
            };
            let get_steam_id: GetSteamIdFn =
                std::mem::transmute(symbol!(b"SteamAPI_ISteamUser_GetSteamID\0"));
            let get_stats: GetStatsFn = match optional!(b"SteamAPI_SteamUserStats_v013\0")
                .or_else(|| optional!(b"SteamAPI_SteamUserStats_v012\0"))
            {
                Some(stats) => std::mem::transmute::<*const c_void, GetStatsFn>(stats),
                None => return Err(missing!(b"SteamAPI_SteamUserStats_v013\0")),
            };
            let set_achievement: SetAchievementFn =
                std::mem::transmute(symbol!(b"SteamAPI_ISteamUserStats_SetAchievement\0"));
            let get_achievement: GetAchievementFn =
                std::mem::transmute(symbol!(b"SteamAPI_ISteamUserStats_GetAchievement\0"));
            let store_stats: StoreStatsFn =
                std::mem::transmute(symbol!(b"SteamAPI_ISteamUserStats_StoreStats\0"));
            let set_stat_int: SetStatIntFn =
                std::mem::transmute(symbol!(b"SteamAPI_ISteamUserStats_SetStatInt32\0"));
            let get_stat_int: GetStatIntFn =
                std::mem::transmute(symbol!(b"SteamAPI_ISteamUserStats_GetStatInt32\0"));
            let set_stat_float: SetStatFloatFn =
                std::mem::transmute(symbol!(b"SteamAPI_ISteamUserStats_SetStatFloat\0"));
            let get_stat_float: GetStatFloatFn =
                std::mem::transmute(symbol!(b"SteamAPI_ISteamUserStats_GetStatFloat\0"));
            let get_storage: GetStorageFn =
                std::mem::transmute(symbol!(b"SteamAPI_SteamRemoteStorage_v016\0"));
            let file_write: FileWriteFn =
                std::mem::transmute(symbol!(b"SteamAPI_ISteamRemoteStorage_FileWrite\0"));
            let file_read: FileReadFn =
                std::mem::transmute(symbol!(b"SteamAPI_ISteamRemoteStorage_FileRead\0"));
            let file_size: GetFileSizeFn =
                std::mem::transmute(symbol!(b"SteamAPI_ISteamRemoteStorage_GetFileSize\0"));
            let file_exists: FileExistsFn =
                std::mem::transmute(symbol!(b"SteamAPI_ISteamRemoteStorage_FileExists\0"));
            let file_delete: FileDeleteFn =
                std::mem::transmute(symbol!(b"SteamAPI_ISteamRemoteStorage_FileDelete\0"));
            Ok(SteamApi {
                _library: library,
                restart,
                init,
                shutdown,
                run_callbacks,
                is_running,
                get_user,
                name_source,
                get_steam_id,
                get_stats,
                set_achievement,
                get_achievement,
                store_stats,
                set_stat_int,
                get_stat_int,
                set_stat_float,
                get_stat_float,
                get_storage,
                file_write,
                file_read,
                file_size,
                file_exists,
                file_delete,
                initialized: false,
            })
        }
    }

    pub fn restart_app_if_necessary(&self, app_id: u32) -> bool {
        unsafe { (self.restart)(app_id) }
    }

    pub fn init(&mut self) -> Result<(), SteamError> {
        if self.initialized {
            return Ok(());
        }
        unsafe { self.init.call()? };
        self.initialized = true;
        Ok(())
    }

    pub fn shutdown(&mut self) {
        if self.initialized {
            unsafe { (self.shutdown)() };
            self.initialized = false;
        }
    }

    pub fn run_callbacks(&self) {
        if self.initialized {
            unsafe { (self.run_callbacks)() };
        }
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    pub fn is_client_running(&self) -> bool {
        unsafe { (self.is_running)() }
    }

    pub fn player_name(&self) -> Option<String> {
        if !self.initialized {
            return None;
        }
        unsafe {
            let user = (self.get_user)();
            if user.is_null() {
                return None;
            }
            match &self.name_source {
                NameSource::User(get_player_name) => c_string(get_player_name(user)),
                NameSource::Friends { friends, persona } => {
                    let friends = friends();
                    if friends.is_null() {
                        return None;
                    }
                    let steam_id = (self.get_steam_id)(user);
                    if steam_id == 0 {
                        return None;
                    }
                    c_string(persona(friends, steam_id))
                }
            }
        }
    }

    pub fn steam_id(&self) -> u64 {
        if !self.initialized {
            return 0;
        }
        unsafe {
            let user = (self.get_user)();
            if user.is_null() {
                return 0;
            }
            (self.get_steam_id)(user)
        }
    }

    pub fn unlock_achievement(&self, name: &str) -> bool {
        if !self.initialized {
            return false;
        }
        let Ok(name) = CString::new(name) else {
            return false;
        };
        unsafe {
            let stats = (self.get_stats)();
            if stats.is_null() {
                return false;
            }
            (self.set_achievement)(stats, name.as_ptr()) && (self.store_stats)(stats)
        }
    }

    pub fn is_achievement_unlocked(&self, name: &str) -> bool {
        if !self.initialized {
            return false;
        }
        let Ok(name) = CString::new(name) else {
            return false;
        };
        unsafe {
            let stats = (self.get_stats)();
            if stats.is_null() {
                return false;
            }
            let mut achieved = false;
            (self.get_achievement)(stats, name.as_ptr(), &mut achieved) && achieved
        }
    }

    pub fn store_stats(&self) -> bool {
        if !self.initialized {
            return false;
        }
        unsafe {
            let stats = (self.get_stats)();
            !stats.is_null() && (self.store_stats)(stats)
        }
    }

    pub fn set_stat_int(&self, name: &str, value: i32) -> bool {
        self.with_stats(name, |stats, name| unsafe {
            (self.set_stat_int)(stats, name, value)
        })
    }

    pub fn get_stat_int(&self, name: &str) -> Option<i32> {
        if !self.initialized {
            return None;
        }
        let name = CString::new(name).ok()?;
        unsafe {
            let stats = (self.get_stats)();
            if stats.is_null() {
                return None;
            }
            let mut value = 0i32;
            (self.get_stat_int)(stats, name.as_ptr(), &mut value).then_some(value)
        }
    }

    pub fn set_stat_float(&self, name: &str, value: f32) -> bool {
        self.with_stats(name, |stats, name| unsafe {
            (self.set_stat_float)(stats, name, value)
        })
    }

    pub fn get_stat_float(&self, name: &str) -> Option<f32> {
        if !self.initialized {
            return None;
        }
        let name = CString::new(name).ok()?;
        unsafe {
            let stats = (self.get_stats)();
            if stats.is_null() {
                return None;
            }
            let mut value = 0f32;
            (self.get_stat_float)(stats, name.as_ptr(), &mut value).then_some(value)
        }
    }

    pub fn write_file(&self, name: &str, data: &[u8]) -> bool {
        if !self.initialized {
            return false;
        }
        let Ok(name) = CString::new(name) else {
            return false;
        };
        unsafe {
            let storage = (self.get_storage)();
            if storage.is_null() {
                return false;
            }
            let Ok(size) = i32::try_from(data.len()) else {
                return false;
            };
            (self.file_write)(storage, name.as_ptr(), data.as_ptr(), size)
        }
    }

    pub fn read_file(&self, name: &str) -> Option<Vec<u8>> {
        if !self.initialized {
            return None;
        }
        let name = CString::new(name).ok()?;
        unsafe {
            let storage = (self.get_storage)();
            if storage.is_null() {
                return None;
            }
            let size = (self.file_size)(storage, name.as_ptr());
            if size <= 0 {
                return None;
            }
            let mut data = vec![0u8; size as usize];
            let read = (self.file_read)(storage, name.as_ptr(), data.as_mut_ptr(), size);
            if read <= 0 {
                return None;
            }
            data.truncate(read as usize);
            Some(data)
        }
    }

    pub fn file_exists(&self, name: &str) -> bool {
        if !self.initialized {
            return false;
        }
        let Ok(name) = CString::new(name) else {
            return false;
        };
        unsafe {
            let storage = (self.get_storage)();
            !storage.is_null() && (self.file_exists)(storage, name.as_ptr())
        }
    }

    pub fn delete_file(&self, name: &str) -> bool {
        if !self.initialized {
            return false;
        }
        let Ok(name) = CString::new(name) else {
            return false;
        };
        unsafe {
            let storage = (self.get_storage)();
            !storage.is_null() && (self.file_delete)(storage, name.as_ptr())
        }
    }

    fn with_stats<F>(&self, name: &str, call: F) -> bool
    where
        F: FnOnce(*mut c_void, *const c_char) -> bool,
    {
        if !self.initialized {
            return false;
        }
        let Ok(name) = CString::new(name) else {
            return false;
        };
        unsafe {
            let stats = (self.get_stats)();
            if stats.is_null() {
                return false;
            }
            call(stats, name.as_ptr())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_names_match_the_platform() {
        let names = library_names();
        assert!(!names.is_empty());
        if cfg!(target_os = "windows") {
            assert!(names.contains(&"steam_api64.dll"));
        } else if cfg!(target_os = "macos") {
            assert!(names.contains(&"libsteam_api.dylib"));
        } else {
            assert!(names.contains(&"libsteam_api.so"));
        }
    }

    #[test]
    fn explicit_library_path_takes_priority() {
        std::env::set_var(STEAM_LIB_ENV, "/tmp/does-not-exist/steam_api");
        let candidates = candidate_paths();
        assert_eq!(
            candidates.first(),
            Some(&PathBuf::from("/tmp/does-not-exist/steam_api"))
        );
        std::env::remove_var(STEAM_LIB_ENV);
    }

    #[test]
    fn loading_reports_a_missing_library() {
        std::env::set_var(STEAM_LIB_ENV, "/tmp/does-not-exist/steam_api");
        let result = SteamApi::try_load();
        assert!(matches!(result, Err(SteamError::LibraryMissing)));
        std::env::remove_var(STEAM_LIB_ENV);
    }

    #[test]
    fn errors_have_messages() {
        assert!(!SteamError::Init("x".to_string()).to_string().is_empty());
        assert!(!SteamError::SymbolMissing("x".to_string())
            .to_string()
            .is_empty());
    }
}
