//! ChamSys Designer account: sign in over HTTP, keep the session cookie in the OS keychain.

use cadcraft_csd::MAX_LIBRARY_BYTES;
use cadcraft_ui_egui::{Account, AccountError, SignedIn};
use serde_json::{Value, json};
use std::time::Duration;
use ureq::Agent;

const DEFAULT_SERVER: &str = "https://csd.atlaspd.com";
const KEYCHAIN_SERVICE: &str = "CADCraft ChamSys Designer";
const COOKIE: &str = "csd_session";

/// Where the session lives: the OS keychain in the app, memory in tests.
trait SessionStore: Send + Sync {
    fn load(&self) -> Option<String>;
    fn save(&self, secret: &str);
    fn clear(&self);
}

struct Keychain(String);

impl Keychain {
    fn entry(&self) -> Option<keyring::Entry> {
        keyring::Entry::new(KEYCHAIN_SERVICE, &self.0).ok()
    }
}

impl SessionStore for Keychain {
    fn load(&self) -> Option<String> {
        self.entry()?.get_password().ok()
    }
    fn save(&self, secret: &str) {
        if self.entry().is_none_or(|e| e.set_password(secret).is_err()) {
            log::warn!("could not store the ChamSys Designer session in the keychain");
        }
    }
    fn clear(&self) {
        if let Some(e) = self.entry() {
            let _ = e.delete_credential();
        }
    }
}

pub struct ServerAccount {
    base: String,
    agent: Agent,
    store: Box<dyn SessionStore>,
}

impl ServerAccount {
    /// The server from `CSD_SERVER`, else the default, with the session in the OS keychain.
    pub fn from_env() -> Self {
        let base = std::env::var("CSD_SERVER").ok().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| DEFAULT_SERVER.into());
        let base = base.trim().trim_end_matches('/').to_string();
        Self::new(base.clone(), Box::new(Keychain(base)))
    }

    fn new(base: String, store: Box<dyn SessionStore>) -> Self {
        // 3xx and 4xx come back as responses: nginx answers a dead session with a redirect.
        // Without root certificates ureq panics on the first https request.
        let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build();
        let agent: Agent = Agent::config_builder()
            .tls_config(tls)
            .timeout_global(Some(Duration::from_secs(15)))
            .max_redirects(0)
            .http_status_as_error(false)
            .build()
            .into();
        Self { base, agent, store }
    }

    fn post(&self, path: &str, email: &str, password: &str) -> Result<ureq::http::Response<ureq::Body>, AccountError> {
        let body = json!({"email": email, "password": password}).to_string();
        self.agent.post(format!("{}{path}", self.base)).header("Content-Type", "application/json").send(body).map_err(unreachable)
    }

    /// `Ok(None)`: the server does not accept this session.
    fn products(&self, token: &str) -> Result<Option<Vec<Value>>, AccountError> {
        let resp = self.agent.get(format!("{}/api/products", self.base)).header("Cookie", format!("{COOKIE}={token}")).call().map_err(unreachable)?;
        let status = resp.status().as_u16();
        if status == 401 || (300..400).contains(&status) {
            return Ok(None);
        }
        if status != 200 {
            return Err(AccountError::Server(format!("the server answered {status}")));
        }
        let bytes = resp
            .into_body()
            .with_config()
            .limit(MAX_LIBRARY_BYTES as u64)
            .read_to_vec()
            .map_err(|_| AccountError::Server("the product list could not be read".into()))?;
        let mut doc: Value = serde_json::from_slice(&bytes).map_err(|_| AccountError::Server("the server sent an unexpected reply".into()))?;
        match doc.get_mut("products").map(Value::take) {
            Some(Value::Array(products)) => Ok(Some(products)),
            _ => Err(AccountError::Server("the server sent an unexpected reply".into())),
        }
    }
}

fn unreachable(_: ureq::Error) -> AccountError {
    AccountError::Server("could not reach the server".into())
}

/// The `csd_session` value of a response's `Set-Cookie` headers.
fn session_token(resp: &ureq::http::Response<ureq::Body>) -> Option<String> {
    resp.headers().get_all("set-cookie").iter().filter_map(|v| v.to_str().ok()).find_map(|c| {
        let token = c.strip_prefix(COOKIE)?.strip_prefix('=')?.split(';').next()?;
        let ok = !token.is_empty() && token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        ok.then(|| token.to_string())
    })
}

fn stored(store: &dyn SessionStore) -> Option<(String, String)> {
    let v: Value = serde_json::from_str(&store.load()?).ok()?;
    Some((v.get("email")?.as_str()?.to_string(), v.get("token")?.as_str()?.to_string()))
}

impl Account for ServerAccount {
    fn resume(&self) -> Result<Option<SignedIn>, AccountError> {
        let Some((email, token)) = stored(&*self.store) else { return Ok(None) };
        match self.products(&token)? {
            Some(products) => Ok(Some(SignedIn { email, products })),
            None => {
                self.store.clear();
                Ok(None)
            }
        }
    }

    fn sign_in(&self, email: &str, password: &str) -> Result<SignedIn, AccountError> {
        let resp = self.post("/api/account/login", email, password)?;
        match resp.status().as_u16() {
            204 => {}
            400 | 401 => return Err(AccountError::Invalid),
            403 => {
                let body = resp.into_body().with_config().limit(4096).read_to_string().unwrap_or_default();
                let reason = serde_json::from_str::<Value>(&body).ok().and_then(|v| v.get("error")?.as_str().map(str::to_owned));
                return Err(match reason.as_deref() {
                    Some("pending") => AccountError::Pending,
                    Some("disabled") => AccountError::Disabled,
                    _ => AccountError::Server("the server answered 403".into()),
                });
            }
            429 => return Err(AccountError::Server("too many attempts, try again in a minute".into())),
            s => return Err(AccountError::Server(format!("the server answered {s}"))),
        }
        let token = session_token(&resp).ok_or_else(|| AccountError::Server("the server sent no session".into()))?;
        let products = self.products(&token)?.ok_or_else(|| AccountError::Server("the server did not accept the new session".into()))?;
        self.store.save(&json!({"email": email, "token": token}).to_string());
        Ok(SignedIn { email: email.to_string(), products })
    }

    fn request_access(&self, email: &str, password: &str) -> Result<(), AccountError> {
        match self.post("/api/account/request", email, password)?.status().as_u16() {
            202 => Ok(()),
            409 => Err(AccountError::Exists),
            400 => Err(AccountError::Server("enter a valid email and a password of 8 or more characters".into())),
            429 => Err(AccountError::Server("too many attempts, try again in a minute".into())),
            s => Err(AccountError::Server(format!("the server answered {s}"))),
        }
    }

    fn sign_out(&self) {
        let session = stored(&*self.store);
        self.store.clear();
        if let Some((_, token)) = session {
            let _ = self.agent.post(format!("{}/api/account/logout", self.base)).header("Cookie", format!("{COOKIE}={token}")).send_empty();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Memory(Arc<Mutex<Option<String>>>);

    impl SessionStore for Memory {
        fn load(&self) -> Option<String> {
            self.0.lock().ok()?.clone()
        }
        fn save(&self, secret: &str) {
            if let Ok(mut g) = self.0.lock() {
                *g = Some(secret.into());
            }
        }
        fn clear(&self) {
            if let Ok(mut g) = self.0.lock() {
                *g = None;
            }
        }
    }

    type Server = (String, Arc<Mutex<Vec<String>>>, std::thread::JoinHandle<()>);

    /// Answers one canned response per connection, in order; returns the base URL and the raw requests seen.
    fn serve(responses: Vec<String>) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let handle = std::thread::spawn(move || {
            for canned in responses {
                let (mut s, _) = listener.accept().unwrap();
                let mut req = Vec::new();
                let mut buf = [0u8; 1024];
                while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = s.read(&mut buf).unwrap();
                    if n == 0 {
                        break;
                    }
                    req.extend_from_slice(&buf[..n]);
                }
                log.lock().unwrap().push(String::from_utf8_lossy(&req).into_owned());
                s.write_all(canned.as_bytes()).unwrap();
            }
        });
        (base, seen, handle)
    }

    fn account(base: &str, store: &Memory) -> ServerAccount {
        ServerAccount::new(base.into(), Box::new(store.clone()))
    }

    fn reply(status: &str, body: &str) -> String {
        format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
    }

    fn login_ok() -> String {
        "HTTP/1.1 204 No Content\r\nSet-Cookie: csd_session=tok_1-A; Path=/; HttpOnly\r\nConnection: close\r\n\r\n".into()
    }

    fn products() -> String {
        reply("200 OK", r#"{"products":[{"id":"a"},{"x":1}]}"#)
    }

    fn redirect() -> String {
        "HTTP/1.1 302 Found\r\nLocation: /login\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into()
    }

    fn with_session() -> Memory {
        let store = Memory::default();
        store.save(r#"{"email":"a@b.co","token":"tok_1-A"}"#);
        store
    }

    #[test]
    fn sign_in_stores_session_and_returns_products() {
        let (base, seen, h) = serve(vec![login_ok(), products()]);
        let store = Memory::default();
        let got = account(&base, &store).sign_in("a@b.co", "hunter22").unwrap();
        h.join().unwrap();
        assert_eq!(got.email, "a@b.co");
        assert_eq!(got.products, vec![json!({"id": "a"}), json!({"x": 1})]);
        assert_eq!(stored(&store), Some(("a@b.co".into(), "tok_1-A".into())));
        let seen = seen.lock().unwrap();
        assert!(seen[0].starts_with("POST /api/account/login"));
        assert!(seen[1].contains("csd_session=tok_1-A"));
    }

    #[test]
    fn sign_in_failures_map_to_errors() {
        let busy = AccountError::Server("too many attempts, try again in a minute".into());
        let cases = [
            (reply("401 Unauthorized", r#"{"error":"invalid"}"#), AccountError::Invalid),
            (reply("400 Bad Request", "{}"), AccountError::Invalid),
            (reply("403 Forbidden", r#"{"error":"pending"}"#), AccountError::Pending),
            (reply("403 Forbidden", r#"{"error":"disabled"}"#), AccountError::Disabled),
            (reply("429 Too Many Requests", ""), busy),
            (reply("500 Oops", "x"), AccountError::Server("the server answered 500".into())),
        ];
        for (canned, want) in cases {
            let (base, _, h) = serve(vec![canned]);
            let store = Memory::default();
            assert_eq!(account(&base, &store).sign_in("a@b.co", "pw").unwrap_err(), want);
            h.join().unwrap();
            assert!(store.load().is_none());
        }
    }

    #[test]
    fn sign_in_without_a_session_cookie_fails() {
        let (base, _, h) = serve(vec!["HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n".into()]);
        let store = Memory::default();
        assert!(matches!(account(&base, &store).sign_in("a@b.co", "pw"), Err(AccountError::Server(_))));
        h.join().unwrap();
        assert!(store.load().is_none());
    }

    #[test]
    fn https_failure_is_an_error_not_a_panic() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("https://{}", listener.local_addr().unwrap());
        let h = std::thread::spawn(move || drop(listener.accept()));
        assert!(matches!(account(&base, &Memory::default()).sign_in("a@b.co", "pw"), Err(AccountError::Server(_))));
        h.join().unwrap();
    }

    #[test]
    fn request_access_maps_status() {
        let short = AccountError::Server("enter a valid email and a password of 8 or more characters".into());
        let cases =
            [(reply("202 Accepted", ""), Ok(())), (reply("409 Conflict", ""), Err(AccountError::Exists)), (reply("400 Bad Request", ""), Err(short))];
        for (canned, want) in cases {
            let (base, _, h) = serve(vec![canned]);
            assert_eq!(account(&base, &Memory::default()).request_access("a@b.co", "password1"), want);
            h.join().unwrap();
        }
    }

    #[test]
    fn resume_with_valid_session() {
        let (base, seen, h) = serve(vec![products()]);
        let store = with_session();
        let got = account(&base, &store).resume().unwrap().unwrap();
        h.join().unwrap();
        assert_eq!(got.email, "a@b.co");
        assert_eq!(got.products.len(), 2);
        assert!(seen.lock().unwrap()[0].contains("csd_session=tok_1-A"));
        assert!(store.load().is_some());
    }

    #[test]
    fn resume_redirect_drops_the_session() {
        let (base, _, h) = serve(vec![redirect()]);
        let store = with_session();
        assert_eq!(account(&base, &store).resume(), Ok(None));
        h.join().unwrap();
        assert!(store.load().is_none());
    }

    #[test]
    fn resume_unreachable_keeps_the_session() {
        let (base, _, h) = serve(vec![]);
        h.join().unwrap(); // listener dropped: connection refused
        let store = with_session();
        assert!(matches!(account(&base, &store).resume(), Err(AccountError::Server(_))));
        assert!(store.load().is_some());
    }

    #[test]
    fn resume_without_session_is_none() {
        assert_eq!(account("http://127.0.0.1:1", &Memory::default()).resume(), Ok(None));
    }

    #[test]
    fn resume_rejects_a_reply_without_products() {
        let (base, _, h) = serve(vec![reply("200 OK", r#"{"nope":[]}"#)]);
        assert!(matches!(account(&base, &with_session()).resume(), Err(AccountError::Server(_))));
        h.join().unwrap();
    }

    #[test]
    fn sign_out_forgets_the_session_and_tells_the_server() {
        let (base, seen, h) = serve(vec![reply("204 No Content", "")]);
        let store = with_session();
        account(&base, &store).sign_out();
        h.join().unwrap();
        assert!(store.load().is_none());
        let seen = seen.lock().unwrap();
        assert!(seen[0].starts_with("POST /api/account/logout") && seen[0].contains("csd_session=tok_1-A"));
    }
}
