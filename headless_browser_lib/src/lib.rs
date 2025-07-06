use cached::proc_macro::once;

/// Chrome configuration.
pub mod conf;
/// Chrome json modifiers.
mod modify;
/// Proxy forwarder TCP to chrome instances.
pub mod proxy;
/// Chrome renderer configuration.
mod render_conf;

use conf::{
    CACHEABLE, CHROME_ADDRESS, CHROME_ARGS, CHROME_INSTANCES, CHROME_PATH, DEBUG_JSON,
    DEFAULT_PORT, DEFAULT_PORT_SERVER, ENDPOINT, HOST_NAME, IS_HEALTHY, LAST_CACHE,
    LIGHTPANDA_ARGS, LIGHT_PANDA, TARGET_REPLACEMENT,
};
use core::sync::atomic::Ordering;
use http_body_util::Full;
use hyper::{
    body::{Bytes, Incoming},
    server::conn::http1,
    service::service_fn,
    Method, Request, Response, StatusCode,
};
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::process::Command;
use tokio::{
    net::{TcpListener, TcpStream},
    signal,
};

use std::time::Duration;
use tokio::time::{sleep, timeout};

/// Empty default response without a 'webSocketDebuggerUrl'.
const EMPTY_RESPONSE: Bytes = Bytes::from_static(
    br#"{
   "Browser": "",
   "Protocol-Version": "",
   "User-Agent": "",
   "V8-Version": "",
   "WebKit-Version": "",
   "webSocketDebuggerUrl": ""
}"#,
);

/// Attempt the connection.
async fn connect_with_retries(address: &str) -> Option<TcpStream> {
    let mut attempts = 0;
    let mut connection_failed = false;

    loop {
        // 15s is the default restart timeout chrome flag.
        match timeout(Duration::from_secs(15), TcpStream::connect(address)).await {
            Ok(Ok(stream)) => return Some(stream),
            Ok(Err(e)) => {
                attempts += 1;
                // connection refused means the instance is not alive.
                if !e.kind().eq(&std::io::ErrorKind::ConnectionRefused) {
                    tracing::warn!("Failed to connect: {}. Attempt {} of 20", e, attempts);
                } else {
                    if !connection_failed {
                        connection_failed = true;
                    }
                    // empty prevent connections retrying
                    if attempts >= 10 && CHROME_INSTANCES.is_empty() {
                        tracing::warn!("ConnectionRefused: {}. Attempt {} of 8", e, attempts);
                        return None;
                    }
                }
            }
            Err(_) => {
                attempts += 1;
                tracing::error!("Connection attempt timed out. Attempt {} of 20", attempts);
            }
        }

        if attempts >= 20 {
            return None;
        }

        let rng = rand::random_range(if connection_failed {
            80..=150
        } else {
            150..=250
        });

        sleep(Duration::from_millis(rng)).await;
    }
}

/// Shutdown the chrome instance by process id.
#[cfg(target_os = "windows")]
pub fn shutdown(pid: &u32) {
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .spawn();
}

/// Shutdown the chrome instance by process id.
#[cfg(not(target_os = "windows"))]
pub fn shutdown(pid: &u32) {
    let _ = Command::new("kill").args(["-9", &pid.to_string()]).spawn();
}

#[cfg(test)]
/// Arguments to test headless without any extra args. Only applies during 'cargo test'.
pub fn get_chrome_args_test() -> [&'static str; 6] {
    *crate::conf::CHROME_ARGS_TEST
}

/// Split a comma-separated string into args, preserving commas inside quotes.
fn smart_split_args(arg_str: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for c in arg_str.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                current.push(c);
            }
            ',' if !in_quotes => {
                if !current.trim().is_empty() {
                    result.push(current.trim().to_string());
                }
                current.clear();
            }
            _ => current.push(c),
        }
    }

    if !current.trim().is_empty() {
        result.push(current.trim().to_string());
    }

    result
}

/// Get the env arguments.
fn get_env_args(env_key: &str) -> Vec<String> {
    std::env::var(env_key)
        .ok()
        .map(|s| smart_split_args(&s))
        .unwrap_or_default()
}

#[cfg(not(test))]
/// Arguments to test headless without any extra args. Only applies during 'cargo test'.
pub fn get_chrome_args_test() -> [&'static str; crate::conf::PERF_ARGS] {
    *crate::conf::CHROME_ARGS
}

/// Fork a chrome process.
pub fn fork(port: Option<u32>) -> String {
    let chrome_args = get_env_args("CHROME_ARGS");

    let id = if !*LIGHT_PANDA {
        let mut command = Command::new(&*CHROME_PATH);

        let cmd = if *crate::conf::TEST_NO_ARGS {
            let mut chrome_args = get_chrome_args_test().map(|e| e.to_string());
            if !CHROME_ADDRESS.is_empty() {
                chrome_args[0] =
                    format!("--remote-debugging-address={}", &CHROME_ADDRESS.to_string());
            }
            if let Some(port) = port {
                chrome_args[1] = format!("--remote-debugging-port={}", &port.to_string());
            }
            command.args(&chrome_args)
        } else {
            let mut chrome_args = CHROME_ARGS.map(|e| e.to_string());

            if !CHROME_ADDRESS.is_empty() {
                chrome_args[0] =
                    format!("--remote-debugging-address={}", &CHROME_ADDRESS.to_string());
            }

            if let Some(port) = port {
                chrome_args[1] = format!("--remote-debugging-port={}", &port.to_string());
            }

            command.args(&chrome_args)
        };

        let cmd = if !chrome_args.is_empty() {
            cmd.args(chrome_args)
        } else {
            cmd
        };

        let id = match cmd.spawn() {
            Ok(child) => {
                let cid = child.id();
                tracing::info!("Chrome PID: {}", cid);
                cid
            }
            Err(e) => {
                tracing::error!("{} command didn't start {:?}", &*CHROME_PATH, e);
                0
            }
        };

        id
    } else {
        let panda_args = LIGHTPANDA_ARGS.map(|e| e.to_string());
        let mut command = Command::new(&*CHROME_PATH);

        let host = panda_args[0].replace("--host=", "");
        let port = panda_args[1].replace("--port=", "");
        let cmd = command.args(["--port", &port, "--host", &host]);

        let cmd = if !chrome_args.is_empty() {
            cmd.args(chrome_args)
        } else {
            cmd
        };

        let id = if let Ok(child) = cmd.spawn() {
            let cid = child.id();

            tracing::info!("Chrome PID: {}", cid);

            cid
        } else {
            tracing::error!("chrome command didn't start");
            0
        };

        id
    };

    CHROME_INSTANCES.insert(id.into());

    id.to_string()
}

/// Get json endpoint for chrome instance proxying.
async fn version_handler_bytes_base(endpoint_path: Option<&str>) -> Option<Bytes> {
    use http_body_util::BodyExt;

    let url = endpoint_path
        .unwrap_or(&ENDPOINT.as_str())
        .parse::<hyper::Uri>()
        .expect("valid chrome endpoint");

    let req = Request::builder()
        .method(Method::GET)
        .uri("/json/version")
        .header(
            hyper::header::HOST,
            url.authority()
                .map_or_else(|| "localhost".to_string(), |f| f.as_str().to_string()),
        )
        .header(hyper::header::CONTENT_TYPE, "application/json")
        .header(hyper::header::CONNECTION, "keep-alive")
        .body(http_body_util::Empty::<Bytes>::new())
        .expect("Failed to build the request");

    let host = url.host().expect("uri has no host");
    let port = url.port_u16().unwrap_or(80);

    let address = format!("{}:{}", host, port);

    let resp = if let Some(stream) = connect_with_retries(&address).await {
        let io = TokioIo::new(stream);

        if let Ok((mut client, conn)) = hyper::client::conn::http1::handshake(io).await {
            tokio::task::spawn(async move {
                if let Err(err) = conn.await {
                    tracing::error!("Connection failed: {:?}", err);
                }
            });

            match client.send_request(req).await {
                Ok(mut resp) => {
                    IS_HEALTHY.store(true, Ordering::Relaxed);

                    let mut bytes_mut = vec![];

                    while let Some(next) = resp.frame().await {
                        if let Ok(frame) = next {
                            if let Some(chunk) = frame.data_ref() {
                                bytes_mut.extend(chunk);
                            }
                        }
                    }

                    if !HOST_NAME.is_empty() {
                        let body = modify::modify_json_output(bytes_mut.into());
                        Some(body)
                    } else {
                        Some(bytes_mut.into())
                    }
                }
                _ => {
                    IS_HEALTHY.store(false, Ordering::Relaxed);
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    resp
}

/// Get json endpoint for chrome instance proxying.
#[once(option = true, sync_writes = true, time = 10)]
async fn version_handler_bytes(endpoint_path: Option<&str>) -> Option<Bytes> {
    version_handler_bytes_base(endpoint_path).await
}

/// Health check handler
async fn health_check_handler() -> Result<Response<Full<Bytes>>, Infallible> {
    if IS_HEALTHY.load(Ordering::Relaxed) {
        Ok(Response::new(Full::new(Bytes::from("healthy"))))
    } else {
        let mut response = Response::new(Full::new(Bytes::from("unhealthy")));
        *response.status_mut() = StatusCode::SERVICE_UNAVAILABLE;

        Ok(response)
    }
}

/// Fork handler.
async fn fork_handler(port: Option<u32>) -> Result<Response<Full<Bytes>>, Infallible> {
    let pid = fork(port);
    let pid = format!("Forked process with pid: {}", pid);

    Ok(Response::new(Full::new(Bytes::from(pid))))
}

/// Json version handler.
async fn json_version_handler(
    endpoint_path: Option<&str>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let mut attempts = 0;
    let mut body: Option<Bytes> = None;
    let mut checked_empty = false;

    // check if the instances are alive.
    while attempts < 10 && body.is_none() && !CHROME_INSTANCES.is_empty() {
        body = if CACHEABLE.load(Ordering::Relaxed) {
            version_handler_bytes(endpoint_path).await
        } else {
            version_handler_bytes_base(endpoint_path).await
        };

        if body.is_none() {
            // check the first instance.
            if !checked_empty {
                checked_empty = true;
                if CHROME_INSTANCES.is_empty() {
                    break;
                }
            }
            attempts += 1;

            let rng = rand::random_range(if checked_empty { 10..=50 } else { 20..=80 });

            tokio::time::sleep(Duration::from_millis(rng)).await;
        }
    }

    let empty = body.is_none();
    let body = body.unwrap_or_else(|| EMPTY_RESPONSE);

    if *DEBUG_JSON {
        tracing::info!("{:?}", body);
    }

    let mut resp = Response::new(Full::new(body));

    resp.headers_mut().insert(
        hyper::header::CONTENT_TYPE,
        hyper::header::HeaderValue::from_static("application/json"), // body has to be json or parse will fail.
    );

    if empty {
        *resp.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
    }

    Ok(resp)
}

/// Shutdown all the chrome instances launched.
pub async fn shutdown_instances() {
    for pid in CHROME_INSTANCES.iter() {
        shutdown(&pid);
    }
    CHROME_INSTANCES.clear();
    CACHEABLE.store(false, std::sync::atomic::Ordering::Relaxed);
}

/// Shutdown handler.
async fn shutdown_handler() -> Result<Response<Full<Bytes>>, Infallible> {
    shutdown_instances().await;

    Ok(Response::new(Full::new(Bytes::from(
        "Shutdown successful.",
    ))))
}

/// Request handler.
async fn request_handler(req: Request<Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    match (req.method(), req.uri().path()) {
        (&Method::GET, "/health") => health_check_handler().await,
        (&Method::GET, "/") => health_check_handler().await,
        (&Method::POST, "/fork") => fork_handler(None).await,
        (&Method::POST, path) if path.starts_with("/fork/") => {
            if let Some(port) = path.split('/').nth(2) {
                if let Ok(port) = port.parse::<u32>() {
                    fork_handler(Some(port)).await
                } else {
                    let message = Response::new(Full::new(Bytes::from("Invalid port argument")));

                    Ok(message)
                }
            } else {
                let message = Response::new(Full::new(Bytes::from("Invalid path")));

                Ok(message)
            }
        }
        // we only care about the main /json/version for 9223 for the proxy forwarder.
        (&Method::GET, "/json/version") => json_version_handler(None).await,
        (&Method::POST, "/shutdown") => shutdown_handler().await,
        _ => {
            let mut resp = Response::new(Full::new(Bytes::from("Not Found")));

            *resp.status_mut() = StatusCode::NOT_FOUND;

            Ok(resp)
        }
    }
}

/// Launch chrome, start the server, and proxy for management.
pub async fn run_main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let auto_start = std::env::args().nth(3).unwrap_or_else(|| {
        if std::env::var("CHROME_INIT").unwrap_or("true".into()) == "true" {
            "init".into()
        } else {
            "ignore".into()
        }
    });

    if auto_start == "init" {
        fork(Some(*DEFAULT_PORT));
    }

    let addr = SocketAddr::new(
        std::net::IpAddr::V4(std::net::Ipv4Addr::new(0, 0, 0, 0)),
        *DEFAULT_PORT_SERVER,
    );

    let listener = TcpListener::bind(addr).await.expect("connection");

    let make_svc = async move {
        let builder_options = std::sync::Arc::new(
            http1::Builder::new()
                .preserve_header_case(true)
                .title_case_headers(true)
                .header_read_timeout(None)
                .half_close(true)
                .auto_date_header(false)
                .to_owned(),
        );

        loop {
            if let Ok((tcp, _)) = listener.accept().await {
                let builder_options = builder_options.clone();

                tokio::task::spawn(async move {
                    let io = TokioIo::new(tcp);
                    if let Err(err) = builder_options
                        .serve_connection(io, service_fn(request_handler))
                        .await
                    {
                        eprintln!("Error serving connection: {:?}", err);
                    }
                });
            }
        }
    };

    println!(
        "Chrome server running on {}:{}",
        if CHROME_ADDRESS.is_empty() {
            "localhost"
        } else {
            &CHROME_ADDRESS
        },
        DEFAULT_PORT_SERVER.to_string()
    );

    tokio::select! {
        _ = make_svc => Ok(()),
        _ = crate::proxy::proxy::run_proxy() =>  Ok(()),
        _ = signal::ctrl_c() => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chrome_args_parsing() {
        let input =
            r#"--headless,--disable-gpu,--disable-features="Feature1,Feature2",--no-sandbox"#;
        let expected = vec![
            "--headless",
            "--disable-gpu",
            r#"--disable-features="Feature1,Feature2""#,
            "--no-sandbox",
        ];

        let result = smart_split_args(input);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_chrome_args_with_extra_spaces() {
        let input = r#" --foo , --bar="x, y" , --baz "#;
        let expected = vec!["--foo", r#"--bar="x, y""#, "--baz"];

        let result = smart_split_args(input);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_empty_string() {
        let input = "";
        let result = smart_split_args(input);
        assert_eq!(result, Vec::<String>::new());
    }

    #[test]
    fn test_no_commas() {
        let input = "--single-arg";
        let result = smart_split_args(input);
        assert_eq!(result, vec!["--single-arg"]);
    }

    #[test]
    fn test_nested_quotes_not_supported() {
        let input = r#"--arg="quoted \"inner\" text",--next"#;
        let result = smart_split_args(input);
        assert_eq!(result, vec![r#"--arg="quoted \"inner\" text""#, "--next"]);
    }
}
