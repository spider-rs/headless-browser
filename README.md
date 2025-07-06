# headless-browser

The `headless-browser` crate offers a scalable solution for managing headless Chrome instances with integrated proxy and server support. This system is designed to optimize resource utilization and enhance performance for large-scale web automation tasks.

## Installation

Make sure you have [Rust](https://www.rust-lang.org/learn/get-started) installed before proceeding.

```sh
cargo install headless_browser
```

## Usage

1. Launches the most recent browser instance, supporting remote proxy connections and a server for `json/version` rewrites to facilitate networking.
2. Allows manual and automatic spawning and termination of multiple headless instances, including error handling functionalities.
3. Provides access to Chrome DevTools Protocol (CDP) WebSocket connections and status, caching values for improved performance.

By default, the instance binds Chrome to `0.0.0.0` when initialized via the API.

Use the `REMOTE_ADDRESS` environment variable to specify the desired address for the Chrome instance, whether local or networked.

The application passes load balancer health checks on port `6000`, providing the status of the Chrome container.

To run Chrome on a load balancer, a companion application is required, which is a primary function of the server.

The default port configuration includes `9223` for Chrome and `9222` for the TCP proxy, in response to `0.0.0.0` not being exposed in recent versions like `HeadlessChrome/131.0.6778.139` and newer.

For web scraping, we suggest using our [Headless Shell Dockerfile](./docker/Dockerfile.headless_shell_playwright) or downloading the [chrome-headless-shell](https://storage.googleapis.com/chrome-for-testing-public/134.0.6998.23/linux64/chrome-headless-shell-linux64.zip). The headless-shell offers significantly faster performance compared to the standard headless mode. Additionally, our Docker image is remarkably compact for the chrome-headless-shell, thanks to a multi-stage build process that installs only the essential components, resulting in a size of just 300 MB compared to Playwright's traditional 1.2 GB default.

---

## API

1. POST: `fork` to start a new chrome instance or use `fork/$port` with the port to startup the instance ex: `curl --location --request POST 'http://localhost:6000/fork/9223'`.
2. POST: `shutdown/$PID` to shutdown the instance. ex: `curl --location --request POST 'http://localhost:6000/shutdown/77057'`
3. POST: `/json/version` get the json info of the chrome instance to connect to web sockets ex: `curl --location --request POST 'http://localhost:6000/json/version'`.

### Curl Examples

`fork`

```sh
curl --location --request POST 'http://localhost:6000/fork'
```

`shutdown`

```sh
curl --location --request POST 'http://localhost:6000/shutdown'
```

`/json/version`

```sh
curl --location --request GET 'http://localhost:6000/json/version' \
--header 'Content-Type: application/json'

# example output
{
   "Browser": "HeadlessChrome/131.0.6778.139",
   "Protocol-Version": "1.3",
   "User-Agent": "Mozilla/5.0 (X11; Linux aarch64) AppleWebKit/537.36 (KHTML, like Gecko) HeadlessChrome/131.0.6778.139 Safari/537.36",
   "V8-Version": "13.1.201.16",
   "WebKit-Version": "537.36 (@c35bbcbd7c2775a12a3f320e05ac0022939b1a8a)",
   "webSocketDebuggerUrl": "ws://127.0.0.1:9222/devtools/browser/43e14f5a-6877-4e2f-846e-ab5801f1b6fc"
}
```

## Args

1. The first arg is the chrome application location example linux `'/opt/google/chrome/chrome'`.
2. The second arg is the chrome address `127.0.0.1`.
3. The third arg you can pass in `init` to auto start chrome on `9222`.

Example to start chrome (all params are optional):

```sh
headless_browser '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' 127.0.0.1 init
# Chrome PID: 87659
# Chrome server at localhost:6000
# DevTools listening on ws://127.0.0.1:9222/devtools/browser/c789f9e0-7f65-495d-baee-243eb454ea15
```

### Docker

You can build this image using the following:

1. Dockerfile (Alpine)
1. Dockerfile.playwright (Playwright Custom Chrome)
1. Dockerfile.headless_shell (Manual)
1. Dockerfile.brave
1. Dockerfile.headless_shell_playwright (Playwright Headless Shell)
1. Dockerfile.xvfb (Virtual Display)
1. Dockerfile.lightpanda

You need to set the env variable passed in as an arg `HOSTNAME_OVERRIDE` to override the docker container and set it to `host.docker.internal`.

#### Manual (WIP)

Use the following to build with docker.
Run the command `./build.sh` to build chrome on the machine with docker.
The build scripts are originally from [docker-headless-shell](https://github.com/chromedp/docker-headless-shell).

### ENV Variables

```sh
# the chrome path on the OS ex: CHROME_PATH=./chrome-headless-shell/mac_arm-132.0.6834.159/chrome-headless-shell-mac-arm64/chrome-headless-shell
CHROME_PATH=
# the remote address of the chrome intance
REMOTE_ADDRESS=
# use brave browser as default. Set the value to true.
BRAVE_ENABLED=
# custom chrome launch arguments
CHROME_ARGS=
```

## Library

You can use the [lib](https://docs.rs/headless_browser_lib/latest/headless_browser_lib/) with `cargo add headless_browser_lib` control the startup and shutdown manually. Below is an example of using the [spider_chrome](https://github.com/spider-rs/spider/tree/main/spider_chrome) project to run CDP commands concurrently fast.

```rust
futures-util = "0.3"
tokio = { version = "1", features = ["full"] }
spider_chrome = "2"
headless_browser_lib = "0.1"
```

```rust
/// spider_chrome is mapped to chromiumoxide since it was forked and kept the API the same.
use chromiumoxide::{browser::Browser, error::CdpError};
use futures_util::stream::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tokio::spawn(headless_browser_lib::run_main()); // spawn main server, proxy, and headless.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await; // give a slight delay for now until we use a oneshot.
    let start = std::time::Instant::now();

    let (browser, mut handler) =
         // port 6000 - main server entry for routing correctly across networks.
        Browser::connect_with_config("http://127.0.0.1:6000/json/version", Default::default())
            .await?;

    let handle = tokio::task::spawn(async move {
        while let Some(h) = handler.next().await {
            if h.is_err() {
                break;
            }
        }
    });

    let navigate_page = async |u: &str| -> Result<String, CdpError> {
        let page = browser.new_page(u).await?;
        let html = page.wait_for_navigation().await?.content().await?;
        Ok::<String, CdpError>(html)
    };

    let (spider_html, example_html) = tokio::join!(
        navigate_page("https://spider.cloud"),
        navigate_page("https://example.com")
    );

    browser.close().await?;
    let _ = handle.await;

    let spider_html = spider_html?;
    let example_html = example_html?;

    let elasped = start.elapsed();

    browser.close().await?;
    let _ = handle.await;

    println!("===\n{}\n{}\n===", "spider.cloud", spider_html);
    println!("===\n{}\n{}\n===", "example.com", example_html);
    println!("Time took: {:?}", elasped);

    Ok(())
}
```

## Testing and Benchmarks

View the [benches](./benches/README.md) to see the performance between browsers for headless.

## Flags

Use the flag `physical_gpu` to enable physical gpu rendering.

## License

MIT
