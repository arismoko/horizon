//! Vanilla's server management protocol (docs/server.md, section 8): JSON-RPC 2.0 over a WebSocket, which
//! `management-server-*` in server.properties (`[properties]` in server.toml) turns on, as vanilla 26.3's
//! `net.minecraft.server.jsonrpc` serves it: the same authentication (`AuthenticationHandler`), the same
//! errors and parameter checks (`Connection`, `IncomingRpcMethod`, `JsonRPCErrors`). The supervisor serves it,
//! so it answers while the game restarts. What `rpc.discover` lists is in `management_schema.rs`; the other
//! methods answer that they aren't in Horizon yet.

use crate::{
    api::{Phase, Shared},
    http::{self, Request, Response},
    management_schema as schema,
    websocket::Socket,
};
use horizon_server_files::properties::{ServerProperties, SystemEntropy, generate_secret_key};
use serde_json::{Value, json};
use std::{
    net::{SocketAddr, TcpListener, ToSocketAddrs},
    path::Path,
    sync::Arc,
};

/// The management server's settings, read from server.properties as vanilla's `JsonRpc.create` reads them.
struct Settings {
    host: String,
    port: i32,
    secret: String,
    allowed_origins: Vec<String>,
}

/// `SecurityConfig.isValid`: 40 letters and digits.
fn valid_secret(secret: &str) -> bool {
    secret.len() == 40 && secret.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// The settings when the server is on. Vanilla writes the secret it generates into server.properties; here
/// it goes into server.toml's `[properties]` too, which writes server.properties.
fn settings(dir: &Path) -> Result<Option<Settings>, String> {
    let config = crate::config::Config::load(dir)?;
    config
        .write_properties(dir)
        .map_err(|e| format!("server.properties: {e}"))?;
    let file = crate::config::read_properties(&dir.join(crate::config::PROPERTIES))?;
    let generated = file.get("management-server-secret").is_none();
    let p = ServerProperties::new(file, &mut SystemEntropy::default())?;
    if !p.management_server_enabled {
        return Ok(None);
    }
    let mut secret = p.management_server_secret;
    if generated {
        secret = generate_secret_key(&mut SystemEntropy::default());
        crate::config::update_all(
            dir,
            &[(
                "properties".into(),
                "management-server-secret".into(),
                toml::Value::String(secret.clone()),
            )],
        )?;
    }
    if !valid_secret(&secret) {
        return Err("Invalid management server secret, must be 40 alphanumeric characters".into());
    }
    if p.management_server_tls_enabled {
        return Err(if p.management_server_tls_keystore.is_empty() {
            "Failed to configure TLS for the server management protocol: TLS is enabled but keystore is not \
             configured (or set management-server-tls-enabled=false)"
        } else {
            "Horizon can't serve the management protocol over TLS yet: set management-server-tls-enabled=false"
        }
        .into());
    }
    Ok(Some(Settings {
        host: p.management_server_host,
        port: p.management_server_port,
        secret,
        allowed_origins: p
            .management_server_allowed_origins
            .split(',')
            .map(str::to_owned)
            .collect(),
    }))
}

/// Starts the management server if server.properties turns it on. A wrong secret or TLS setting stops the
/// server from starting, as it stops vanilla's.
pub fn start(shared: Arc<Shared>) -> Result<Option<SocketAddr>, String> {
    let Some(settings) = settings(&shared.dir)? else {
        return Ok(None);
    };
    let address = (settings.host.as_str(), settings.port as u16)
        .to_socket_addrs()
        .ok()
        .and_then(|mut a| a.next())
        .ok_or_else(|| format!("management-server-host: can't resolve {}", settings.host))?;
    let listener = TcpListener::bind(address)
        .map_err(|e| format!("the management server can't listen on {address}: {e}"))?;
    let address = listener.local_addr().map_err(|e| e.to_string())?;
    let settings = Arc::new(settings);
    http::serve(listener, move |request| {
        handshake(&shared, &settings, request)
    })
    .map_err(|e| e.to_string())?;
    Ok(Some(address))
}

/// `AuthenticationHandler.performSecurityChecks`: the secret as a bearer token, or in the WebSocket
/// subprotocol from an allowed origin. `Ok(true)` when it came in the subprotocol.
fn authenticate(settings: &Settings, request: &Request) -> Result<bool, &'static str> {
    let valid = |key: &str| !key.is_empty() && crate::api::same_token(key, &settings.secret);
    if let Some(token) = request
        .header("authorization")
        .and_then(|h| h.strip_prefix("Bearer "))
    {
        return if valid(token.trim()) {
            Ok(false)
        } else {
            Err("Invalid API key")
        };
    }
    let Some(token) = request
        .header("sec-websocket-protocol")
        .and_then(|h| h.strip_prefix("minecraft-v1,"))
    else {
        return Err("Missing API key");
    };
    let origin = request.header("origin").unwrap_or_default();
    if origin.is_empty() || !settings.allowed_origins.iter().any(|o| o == origin) {
        return Err("Origin Not Allowed");
    }
    if valid(token.trim()) {
        Ok(true)
    } else {
        Err("Invalid API key")
    }
}

fn handshake(shared: &Arc<Shared>, settings: &Settings, request: &Request) -> Response {
    let in_protocol = match authenticate(settings, request) {
        Ok(in_protocol) => in_protocol,
        Err(reason) => {
            let body = format!("{{\"error\":\"Unauthorized\",\"message\":\"{reason}\"}}");
            return Response::text(401, "application/json", body);
        }
    };
    if request.path != "/" {
        return Response::error(404, "the management protocol is served on /");
    }
    let upgrade = request
        .header("upgrade")
        .is_some_and(|u| u.eq_ignore_ascii_case("websocket"));
    let (true, Some(key)) = (upgrade, request.header("sec-websocket-key")) else {
        return Response::error(400, "not a WebSocket upgrade");
    };
    let mut response = Response::text(101, "", "")
        .with("Upgrade", "websocket")
        .with("Connection", "Upgrade")
        .with("Sec-WebSocket-Accept", crate::websocket::accept_key(key));
    if in_protocol {
        response = response.with("Sec-WebSocket-Protocol", "minecraft-v1");
    }
    let shared = shared.clone();
    response.stream = Some(Box::new(move |stream| {
        let Ok(socket) = Socket::new(stream) else {
            return;
        };
        while let Ok(Some(text)) = socket.read_text() {
            match receive(&shared, &text) {
                Some(Ok(reply)) => {
                    if socket.send_text(&reply.to_string()).is_err() {
                        return;
                    }
                }
                Some(Err(())) => break,
                None => {}
            }
        }
        socket.close(1000);
    }));
    response
}

const PARSE_ERROR: i32 = -32700;
const INVALID_REQUEST: i32 = -32600;
const METHOD_NOT_FOUND: i32 = -32601;
pub(crate) const INVALID_PARAMS: i32 = -32602;
pub(crate) const INTERNAL_ERROR: i32 = -32603;

/// `JsonRPCUtils.createError`.
fn error(id: Value, code: i32, data: Option<&str>) -> Value {
    let message = match code {
        PARSE_ERROR => "Parse error",
        INVALID_REQUEST => "Invalid Request",
        METHOD_NOT_FOUND => "Method not found",
        INVALID_PARAMS => "Invalid params",
        _ => "Internal error",
    };
    let mut error = json!({ "code": code, "message": message });
    if let Some(data) = data.filter(|d| !d.trim().is_empty()) {
        error["data"] = json!(data);
    }
    json!({ "jsonrpc": "2.0", "id": id, "error": error })
}

/// `Connection.channelRead0`: the reply to one message, if any. `Err` closes the connection, as vanilla's
/// does when a batch holds something other than an object.
pub fn receive(shared: &Shared, text: &str) -> Option<Result<Value, ()>> {
    let message: Value = match serde_json::from_str(text) {
        Ok(message) => message,
        Err(e) => return Some(Ok(error(Value::Null, PARSE_ERROR, Some(&e.to_string())))),
    };
    Some(Ok(match message {
        Value::Object(object) => return handle(shared, &object).map(Ok),
        Value::Array(batch) => {
            let mut replies = Vec::new();
            for request in batch {
                let Value::Object(object) = request else {
                    return Some(Err(()));
                };
                replies.extend(handle(shared, &object));
            }
            Value::Array(replies)
        }
        _ => error(Value::Null, INVALID_REQUEST, None),
    }))
}

type Object = serde_json::Map<String, Value>;

/// `Connection.handleJsonObject`.
fn handle(shared: &Shared, object: &Object) -> Option<Value> {
    let id = object.get("id");
    let method = match object.get("method") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(n @ (Value::Number(_) | Value::Bool(_))) => Some(n.to_string()),
        Some(_) => {
            let data = "Unknown error handling request - check server logs for stack trace";
            return Some(error(Value::Null, INTERNAL_ERROR, Some(data)));
        }
    };
    let result = object.get("result");
    let failed = object.get("error").filter(|e| e.is_object());
    match (method, result, failed) {
        (Some(method), None, None) => {
            if id.is_some_and(|id| !(id.is_null() || id.is_number() || id.is_string())) {
                let data = "Invalid request id - only String, Number and NULL supported";
                return Some(error(Value::Null, INVALID_REQUEST, Some(data)));
            }
            let reply = dispatch(shared, &method, object.get("params"));
            let id = id?.clone();
            Some(match reply {
                Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                Err((code, data)) => error(id, code, data.as_deref()),
            })
        }
        // Answers to requests: the server sends none yet.
        (None, Some(_), None) if id.is_some() => None,
        (None, None, Some(_)) => None,
        _ => Some(error(
            id.cloned().unwrap_or(Value::Null),
            INVALID_REQUEST,
            None,
        )),
    }
}

pub(crate) type Failure = (i32, Option<String>);

pub(crate) fn invalid_params(data: impl Into<String>) -> Failure {
    (INVALID_PARAMS, Some(data.into()))
}

/// `Identifier.tryParse`: `namespace:path`, `minecraft` when there is no namespace.
fn identifier(name: &str) -> Option<String> {
    let (namespace, path) = name.split_once(':').unwrap_or(("minecraft", name));
    let ok = |s: &str, extra: &[u8]| {
        s.bytes().all(|b| {
            matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.') || extra.contains(&b)
        })
    };
    (ok(namespace, b"") && ok(path, b"/")).then(|| format!("{namespace}:{path}"))
}

/// `Connection.dispatchIncomingRequest` and `IncomingRpcMethod.apply`.
fn dispatch(shared: &Shared, method: &str, params: Option<&Value>) -> Result<Value, Failure> {
    let Some(name) = identifier(method) else {
        return Err((
            INVALID_REQUEST,
            Some(format!("Failed to parse method value: {method}")),
        ));
    };
    let not_found = || {
        (
            METHOD_NOT_FOUND,
            Some(format!("Method not found: {method}")),
        )
    };
    let (short, param) = if name == "minecraft:rpc.discover" {
        ("rpc.discover", None)
    } else {
        let found = name
            .strip_prefix("minecraft:")
            .and_then(schema::incoming)
            .ok_or_else(not_found)?;
        (found.0, found.2)
    };
    // Only `rpc.discover` answers before vanilla's server is up; here, while the game isn't running.
    if short != "rpc.discover" && shared.state.lock().unwrap().phase != Phase::Running {
        return Err((
            INVALID_REQUEST,
            Some(format!(
                "Method cannot be dispatched pre server initialization: {method}"
            )),
        ));
    }
    let _argument = match (param, params) {
        (None, None) => Value::Null,
        (None, Some(Value::Array(a))) if a.is_empty() => Value::Null,
        (None, Some(_)) => return Err(invalid_params("Expected no params, or an empty array")),
        (Some((name, _)), Some(Value::Object(by_name))) => {
            by_name.get(name).cloned().ok_or_else(|| {
                invalid_params(format!(
                    "Params passed by-name, but expected param [{name}] does not exist"
                ))
            })?
        }
        (Some(_), Some(Value::Array(a))) if a.len() == 1 => a[0].clone(),
        (Some(_), Some(Value::Array(_))) => {
            return Err(invalid_params(
                "Expected exactly one element in the params array",
            ));
        }
        (Some(_), _) => return Err(invalid_params("Expected params as array or named")),
    };
    if short == "rpc.discover" {
        return Ok(schema::discover());
    }
    Err((
        INTERNAL_ERROR,
        Some(format!("minecraft:{short} isn't in Horizon yet")),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};

    const SECRET: &str = "abcdefghijABCDEFGHIJ0123456789abcdefghij";

    fn settings() -> Settings {
        Settings {
            host: "127.0.0.1".into(),
            port: 0,
            secret: SECRET.into(),
            allowed_origins: vec!["http://localhost:3000".into()],
        }
    }

    fn request(headers: &[(&str, &str)]) -> Request {
        Request {
            method: "GET".into(),
            path: "/".into(),
            query: String::new(),
            headers: headers
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            body: Vec::new(),
        }
    }

    #[test]
    fn the_secret_comes_as_vanillas_authentication_handler_takes_it() {
        let s = settings();
        let bearer = format!("Bearer {SECRET}");
        let protocol = format!("minecraft-v1, {SECRET}");
        let cases: [(&[(&str, &str)], Result<bool, &str>); 6] = [
            (&[], Err("Missing API key")),
            (&[("Authorization", "Bearer nope")], Err("Invalid API key")),
            (&[("authorization", &bearer)], Ok(false)),
            (
                &[("Sec-WebSocket-Protocol", &protocol)],
                Err("Origin Not Allowed"),
            ),
            (
                &[
                    ("Sec-WebSocket-Protocol", &protocol),
                    ("Origin", "http://evil"),
                ],
                Err("Origin Not Allowed"),
            ),
            (
                &[
                    ("Sec-WebSocket-Protocol", &protocol),
                    ("Origin", "http://localhost:3000"),
                ],
                Ok(true),
            ),
        ];
        for (headers, expected) in cases {
            assert_eq!(authenticate(&s, &request(headers)), expected, "{headers:?}");
        }
        assert!(valid_secret(SECRET) && !valid_secret("short") && !valid_secret(&"é".repeat(20)));
    }

    #[test]
    fn rpc_discover_lists_what_vanilla_registers() {
        let d = schema::discover();
        let methods = d["methods"].as_array().unwrap();
        assert_eq!(methods.len(), 89);
        assert_eq!(methods[0]["name"], "minecraft:allowlist");
        assert_eq!(
            methods[1]["params"],
            json!([{ "name": "players", "required": true,
                     "schema": { "type": "array", "items": { "$ref": "#/components/schemas/player" } } }])
        );
        assert!(
            methods
                .iter()
                .all(|m| m["name"] != "minecraft:rpc.discover")
        );
        assert!(schema::incoming("minecraft:server/status").is_some());
        assert!(schema::incoming("notification/server/started").is_none());
        assert_eq!(d["components"]["schemas"].as_object().unwrap().len(), 14);
    }

    fn shared() -> Arc<Shared> {
        let dir = std::env::temp_dir().join(format!(
            "horizon-management-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Shared::with_install(&dir, None)
    }

    #[test]
    fn requests_fail_as_vanillas_connection_says() {
        let shared = shared();
        let reply = |text: &str| receive(&shared, text);
        let error = |text: &str| reply(text).unwrap().unwrap()["error"].clone();
        assert_eq!(error("{nope")["code"], PARSE_ERROR);
        assert_eq!(
            error("5"),
            json!({ "code": -32600, "message": "Invalid Request" })
        );
        assert_eq!(
            error(r#"{"id":1,"method":"minecraft:nope"}"#),
            json!({ "code": -32601, "message": "Method not found", "data": "Method not found: minecraft:nope" })
        );
        assert_eq!(
            error(r#"{"id":1,"method":"Bad Name"}"#)["data"],
            "Failed to parse method value: Bad Name"
        );
        let bad_id = reply(r#"{"id":{"a":1},"method":"players"}"#)
            .unwrap()
            .unwrap();
        assert_eq!(bad_id["id"], Value::Null);
        assert_eq!(
            bad_id["error"]["data"],
            "Invalid request id - only String, Number and NULL supported"
        );
        assert_eq!(
            error(r#"{"id":"x","method":"players"}"#)["data"],
            "Method cannot be dispatched pre server initialization: players"
        );
        assert_eq!(
            error(r#"{"id":3,"method":"rpc.discover","params":{}}"#)["data"],
            "Expected no params, or an empty array"
        );
        assert_eq!(
            error(r#"{"id":6}"#),
            json!({ "code": -32600, "message": "Invalid Request" })
        );
        // Notifications and answers get no reply; a batch gets one per request.
        assert!(reply(r#"{"method":"rpc.discover"}"#).is_none());
        assert!(reply(r#"{"id":7,"result":1}"#).is_none());
        assert_eq!(reply("[]").unwrap().unwrap(), json!([]));
        let batch = reply(r#"[{"id":4,"method":"rpc.discover","params":[]},{"method":"x"}]"#);
        let batch = batch.unwrap().unwrap();
        assert_eq!(batch.as_array().unwrap().len(), 1);
        assert_eq!(batch[0]["id"], 4);
        assert_eq!(batch[0]["result"]["openrpc"], "1.3.2");
        assert!(reply("[1]").unwrap().is_err());
    }

    /// A client's text frame, masked.
    fn frame(text: &str) -> Vec<u8> {
        let mut f = vec![0x81, 0x80 | 126];
        f.extend_from_slice(&(text.len() as u16).to_be_bytes());
        f.extend_from_slice(&[0, 0, 0, 0]);
        f.extend_from_slice(text.as_bytes());
        f
    }

    #[test]
    fn a_websocket_client_discovers_the_methods() {
        let shared = shared();
        let settings = Arc::new(settings());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        http::serve(listener, move |r| handshake(&shared, &settings, r)).unwrap();
        let mut refused = std::net::TcpStream::connect(address).unwrap();
        write!(
            refused,
            "GET / HTTP/1.1\r\nAuthorization: Bearer wrong\r\n\r\n"
        )
        .unwrap();
        let mut text = String::new();
        refused.read_to_string(&mut text).unwrap();
        assert!(text.starts_with("HTTP/1.1 401 "), "{text}");
        assert!(
            text.ends_with("\r\n\r\n{\"error\":\"Unauthorized\",\"message\":\"Invalid API key\"}")
        );

        let mut stream = std::net::TcpStream::connect(address).unwrap();
        write!(
            stream,
            "GET / HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\
             Authorization: Bearer {SECRET}\r\n\r\n"
        )
        .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut head = Vec::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            head.push(line.trim_end().to_owned());
        }
        assert_eq!(head[0], "HTTP/1.1 101 Switching Protocols");
        assert!(head.contains(&"Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=".to_owned()));
        stream
            .write_all(&frame(
                r#"{"jsonrpc":"2.0","id":1,"method":"rpc.discover"}"#,
            ))
            .unwrap();
        let mut b = [0u8; 2];
        reader.read_exact(&mut b).unwrap();
        assert_eq!(b[0], 0x81);
        let length = match b[1] {
            126 => {
                let mut l = [0u8; 2];
                reader.read_exact(&mut l).unwrap();
                u16::from_be_bytes(l) as usize
            }
            127 => {
                let mut l = [0u8; 8];
                reader.read_exact(&mut l).unwrap();
                u64::from_be_bytes(l) as usize
            }
            n => n as usize,
        };
        let mut payload = vec![0u8; length];
        reader.read_exact(&mut payload).unwrap();
        let reply: Value = serde_json::from_slice(&payload).unwrap();
        assert_eq!(reply["id"], 1);
        assert_eq!(reply["result"], schema::discover());
    }
}
