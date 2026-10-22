use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

const AUTH_TIMEOUT: Duration = Duration::from_secs(10 * 60);

pub enum AuthEvent {
    Callback(String),
    Cancelled,
}

pub struct AuthServer {
    receiver: Receiver<AuthEvent>,
    relay_url: String,
    shutdown: Arc<AtomicBool>,
}

impl AuthServer {
    pub fn start(provider_url: &str) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let token = random_token();
        let page = relay_page(provider_url, &token);
        let (sender, receiver) = channel();
        let shutdown = Arc::new(AtomicBool::new(false));
        let relay_url = format!("http://127.0.0.1:{port}/{token}/");
        let thread_shutdown = Arc::clone(&shutdown);
        let deadline = Instant::now() + AUTH_TIMEOUT;
        std::thread::Builder::new()
            .name("auth-relay".to_string())
            .spawn(move || serve(listener, &token, &page, &sender, &thread_shutdown, deadline))?;
        Ok(Self {
            receiver,
            relay_url,
            shutdown,
        })
    }

    pub fn relay_url(&self) -> &str {
        &self.relay_url
    }

    pub fn drain(&self) -> Option<AuthEvent> {
        match self.receiver.try_recv() {
            Ok(event) => Some(event),
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => None,
        }
    }
}

impl Drop for AuthServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

pub fn open_browser(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("cmd");
        command.args(["/C", "start", ""]);
        command
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut command = std::process::Command::new("xdg-open");
    command.arg(url);
    command.spawn().map(|_| ())
}

fn serve(
    listener: TcpListener,
    token: &str,
    page: &str,
    sender: &Sender<AuthEvent>,
    shutdown: &AtomicBool,
    deadline: Instant,
) {
    let root = format!("/{token}/");
    let callback = format!("/{token}/callback");
    let cancel = format!("/{token}/cancel");
    while !shutdown.load(Ordering::Relaxed) {
        if Instant::now() >= deadline {
            let _ = sender.send(AuthEvent::Cancelled);
            return;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                if handle(stream, &root, page, &callback, &cancel, sender) {
                    return;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => return,
        }
    }
}

fn handle(
    mut stream: TcpStream,
    root: &str,
    page: &str,
    callback: &str,
    cancel: &str,
    sender: &Sender<AuthEvent>,
) -> bool {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let Some(request) = read_request(&mut stream) else {
        return false;
    };
    if request.method == "GET"
        && (request.target == root || request.target == root.trim_end_matches('/'))
    {
        respond(&mut stream, "200 OK", "text/html", page);
        return false;
    }
    if request.method == "POST" && request.target == callback {
        let _ = sender.send(AuthEvent::Callback(request.body));
        respond(&mut stream, "200 OK", "text/html", CALLBACK_PAGE);
        return true;
    }
    if request.method == "POST" && request.target == cancel {
        let _ = sender.send(AuthEvent::Cancelled);
        respond(&mut stream, "200 OK", "text/html", CANCEL_PAGE);
        return true;
    }
    respond(&mut stream, "404 Not Found", "text/plain", "not found");
    false
}

struct Request {
    method: String,
    target: String,
    body: String,
}

fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(index) = find_blank_line(&buffer) {
            break index;
        }
        if buffer.len() > 32 * 1024 {
            return None;
        }
    };
    let head = std::str::from_utf8(&buffer[..header_end]).ok()?;
    let mut lines = head.split("\r\n");
    let mut parts = lines.next()?.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?.to_string();
    let mut length = 0usize;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                length = value.trim().parse().ok()?;
            }
        }
    }
    if length > 128 * 1024 {
        return None;
    }
    let mut body = buffer[header_end..].to_vec();
    while body.len() < length {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(length.min(body.len()));
    Some(Request {
        method,
        target,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

fn find_blank_line(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
}

fn respond(stream: &mut TcpStream, status: &str, content_type: &str, body: &str) {
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\ncontent-type: {content_type}; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.flush();
}

fn relay_page(provider_url: &str, token: &str) -> String {
    let provider = serde_json::to_string(provider_url).unwrap_or_else(|_| "\"\"".to_string());
    let token = serde_json::to_string(token).unwrap_or_else(|_| "\"\"".to_string());
    RELAY_PAGE
        .replace("__PROVIDER_URL__", &provider)
        .replace("__TOKEN__", &token)
}

fn random_token() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};

    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0),
    );
    hasher.write_u32(std::process::id());
    let first = hasher.finish();
    let mut second_hasher = RandomState::new().build_hasher();
    second_hasher.write_u64(first);
    format!("{first:016x}{:016x}", second_hasher.finish())
}

const CALLBACK_PAGE: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>Signed in</title></head><body style=\"margin:0;min-height:100vh;display:grid;place-items:center;background:#111;color:#eee;font:16px system-ui,sans-serif\"><main>Signed in. You can close this tab and return to the game.</main></body></html>";

const CANCEL_PAGE: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>Sign-in cancelled</title></head><body style=\"margin:0;min-height:100vh;display:grid;place-items:center;background:#111;color:#eee;font:16px system-ui,sans-serif\"><main>Sign-in cancelled. You can close this tab.</main></body></html>";

const RELAY_PAGE: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Sign in</title>
<style>
  :root { color-scheme: dark; }
  body { margin: 0; min-height: 100vh; display: grid; place-items: center;
         background: #111; color: #eee; font: 16px/1.5 system-ui, sans-serif; }
  main { max-width: 28rem; padding: 2rem; text-align: center; }
  button { font: inherit; font-weight: 600; padding: 0.7rem 1.4rem; border: 0;
           border-radius: 6px; background: #5865f2; color: #fff; cursor: pointer; }
  button:hover { background: #4752c4; }
  p { color: #aaa; }
</style>
</head>
<body>
<main>
  <h1>Sign in</h1>
  <p id="status">Complete sign-in in your browser, then return to the game.</p>
  <button id="continue">Continue</button>
</main>
<script>
const provider = __PROVIDER_URL__;
const token = __TOKEN__;
const button = document.getElementById("continue");
const status = document.getElementById("status");
const match = provider.match(/\/provider\/([^/?#]+)/);
if (match) {
  button.textContent = "Continue with " + match[1].charAt(0).toUpperCase() + match[1].slice(1);
}
button.focus();
let popup = null;
let settled = false;
button.addEventListener("click", () => {
  if (settled || popup) { return; }
  popup = window.open(provider, "kaplay-native-auth", "popup,width=520,height=760");
  if (!popup) {
    status.textContent = "The sign-in window was blocked. Allow popups and try again.";
    return;
  }
  button.hidden = true;
  status.textContent = "Complete sign-in in the popup window\u2026";
});
window.addEventListener("message", (event) => {
  if (settled) { return; }
  if (popup && event.source !== popup) { return; }
  const data = event.data;
  if (data === null || typeof data !== "object") { return; }
  if (data.user === undefined && data.token === undefined && data.error === undefined) { return; }
  settled = true;
  fetch("/" + token + "/callback", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(data),
  }).then(() => {
    status.textContent = "Signed in. You can close this tab and return to the game.";
    button.hidden = true;
    if (popup && !popup.closed) { popup.close(); }
  }).catch(() => {
    status.textContent = "Could not reach the game. Return to the game and try again.";
  });
});
setInterval(() => {
  if (!settled && popup && popup.closed) {
    settled = true;
    popup = null;
    status.textContent = "Sign-in cancelled. Return to the game and try again.";
    button.hidden = true;
    fetch("/" + token + "/cancel", { method: "POST" }).catch(() => {});
  }
}, 500);
</script>
</body>
</html>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn get(url: &str) -> String {
        let (authority, path) = url
            .strip_prefix("http://")
            .unwrap()
            .split_once('/')
            .unwrap();
        let mut stream = TcpStream::connect(authority).unwrap();
        let request =
            format!("GET /{path} HTTP/1.1\r\nhost: {authority}\r\nconnection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }

    fn post(url: &str, body: &str) -> String {
        let (authority, path) = url
            .strip_prefix("http://")
            .unwrap()
            .split_once('/')
            .unwrap();
        let mut stream = TcpStream::connect(authority).unwrap();
        let request = format!(
            "POST /{path} HTTP/1.1\r\nhost: {authority}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(request.as_bytes()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }

    #[test]
    fn serves_the_relay_page_with_the_provider_url() {
        let server =
            AuthServer::start("https://game.example/auth/provider/discord?token=abc").unwrap();
        let response = get(server.relay_url());
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains(r#""https://game.example/auth/provider/discord?token=abc""#));
        assert!(response.contains("Continue with "));
        assert!(response.contains("window.open(provider"));
        assert!(response.contains("if (!settled && popup && popup.closed)"));
    }

    #[test]
    fn receives_the_callback_payload() {
        let server = AuthServer::start("https://game.example/auth/provider/discord").unwrap();
        let body = r#"{"user":{"id":"1"},"token":"jwt"}"#;
        let response = post(&format!("{}callback", server.relay_url()), body);
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        match server.drain() {
            Some(AuthEvent::Callback(payload)) => assert_eq!(payload, body),
            _ => panic!("expected the callback payload"),
        }
    }

    #[test]
    fn receives_cancellations() {
        let server = AuthServer::start("https://game.example/auth/provider/discord").unwrap();
        let response = post(&format!("{}cancel", server.relay_url()), "");
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(matches!(server.drain(), Some(AuthEvent::Cancelled)));
    }

    #[test]
    fn ignores_unknown_paths() {
        let server = AuthServer::start("https://game.example/auth/provider/discord").unwrap();
        let response = get(&format!("{}../other", server.relay_url()));
        assert!(response.starts_with("HTTP/1.1 404 Not Found"));
        assert!(server.drain().is_none());
    }
}
