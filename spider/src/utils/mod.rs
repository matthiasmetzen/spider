/// Utils to modify the HTTP header.
pub mod header_utils;

use crate::tokio_stream::StreamExt;
use crate::Client;
#[cfg(feature = "cache_chrome_hybrid")]
use http_cache_semantics::{RequestLike, ResponseLike};

use log::{info, log_enabled, Level};
#[cfg(feature = "headers")]
use reqwest::header::HeaderMap;
use reqwest::{Error, Response, StatusCode};

lazy_static! {
    /// Prevent fetching resources beyond the bytes limit.
    static ref MAX_SIZE_BYTES: usize = {
        match std::env::var("SPIDER_MAX_SIZE_BYTES") {
            Ok(b) => {
                const DEFAULT_MAX_SIZE_BYTES: usize = 1_073_741_824; // 1GB in bytes

                let b = b.parse::<usize>().unwrap_or(DEFAULT_MAX_SIZE_BYTES);

                if b == 0 {
                    0
                } else {
                    b.max(1_048_576) // min 1mb
                }
            },
            _ => 0
        }
    };
}

/// Handle protected pages via chrome. This does nothing without the real_browser feature enabled.
#[cfg(all(feature = "chrome", feature = "real_browser"))]
async fn cf_handle(
    b: &mut bytes::Bytes,
    page: &chromiumoxide::Page,
) -> Result<(), chromiumoxide::error::CdpError> {
    use crate::configuration::{WaitFor, WaitForDelay, WaitForIdleNetwork};
    lazy_static! {
        static ref CF_END: &'static [u8; 62] =
            b"target=\"_blank\">Cloudflare</a></div></div></div></body></html>";
        static ref CF_END2: &'static [u8; 72] =
            b"Performance &amp; security by Cloudflare</div></div></div></body></html>";
        static ref CF_HEAD: &'static [u8; 34] = b"<html><head>\n    <style global=\"\">";
        static ref CF_MOCK_FRAME: &'static [u8; 137] = b"<iframe height=\"1\" width=\"1\" style=\"position: absolute; top: 0px; left: 0px; border: none; visibility: hidden;\"></iframe>\n\n</body></html>";
    };

    let cf = CF_END.as_ref();
    let cf2 = CF_END2.as_ref();
    let cn = CF_HEAD.as_ref();
    let cnf = CF_MOCK_FRAME.as_ref();

    if b.ends_with(cf) || b.ends_with(cf2) || b.starts_with(cn) && b.ends_with(cnf) {
        let mut wait_for = WaitFor::default();
        wait_for.delay = WaitForDelay::new(Some(core::time::Duration::from_secs(1))).into();
        wait_for.idle_network =
            WaitForIdleNetwork::new(core::time::Duration::from_secs(8).into()).into();
        page_wait(&page, &Some(wait_for.clone())).await;

        let _ = page
            .evaluate(r###"document.querySelectorAll("iframe").forEach(el=>el.click());"###)
            .await;

        wait_for.page_navigations = true;
        page_wait(&page, &Some(wait_for.clone())).await;

        let next_content = page.content_bytes().await?;

        let next_content = if next_content.ends_with(cf)
            || next_content.ends_with(cf2)
            || next_content.starts_with(cn) && next_content.ends_with(cnf)
        {
            wait_for.delay = WaitForDelay::new(Some(core::time::Duration::from_secs(4))).into();
            page_wait(&page, &Some(wait_for)).await;
            page.content_bytes().await?
        } else {
            next_content
        };

        *b = next_content;
    }

    Ok(())
}

/// Handle cloudflare protected pages via chrome. This does nothing without the real_browser feature enabled.
#[cfg(all(feature = "chrome", not(feature = "real_browser")))]
async fn cf_handle(
    _b: &mut bytes::Bytes,
    _page: &chromiumoxide::Page,
) -> Result<(), chromiumoxide::error::CdpError> {
    Ok(())
}

/// The response of a web page.
#[derive(Debug, Default)]
pub struct PageResponse {
    /// The page response resource.
    pub content: Option<bytes::Bytes>,
    #[cfg(feature = "headers")]
    /// The headers of the response. (Always None if a webdriver protocol is used for fetching.).
    pub headers: Option<HeaderMap>,
    /// The status code of the request.
    pub status_code: StatusCode,
    /// The final url destination after any redirects.
    pub final_url: Option<String>,
    /// The message of the response error if any.
    pub error_for_status: Option<Result<Response, Error>>,
    #[cfg(feature = "chrome")]
    /// The screenshot bytes of the page. The ScreenShotConfig bytes boolean needs to be set to true.
    pub screenshot_bytes: Option<Vec<u8>>,
    #[cfg(feature = "openai")]
    /// The credits used from OpenAI in order.
    pub openai_credits_used: Option<Vec<crate::features::openai_common::OpenAIUsage>>,
    #[cfg(feature = "openai")]
    /// The extra data from the AI, example extracting data etc...
    pub extra_ai_data: Option<Vec<crate::page::AIResults>>,
}

/// wait for event with timeout
#[cfg(feature = "chrome")]
pub async fn wait_for_event<T>(page: &chromiumoxide::Page, timeout: Option<core::time::Duration>)
where
    T: chromiumoxide::cdp::IntoEventKind + Unpin + std::fmt::Debug,
{
    match page.event_listener::<T>().await {
        Ok(mut events) => {
            let wait_until = async {
                loop {
                    let sleep = tokio::time::sleep(tokio::time::Duration::from_millis(500));
                    tokio::pin!(sleep);
                    tokio::select! {
                        _ = &mut sleep => break,
                        v = events.next() => {
                            if !v.is_none () {
                                break;
                            }
                        }
                    }
                }
            };
            match timeout {
                Some(timeout) => if let Err(_) = tokio::time::timeout(timeout, wait_until).await {},
                _ => wait_until.await,
            }
        }
        _ => (),
    }
}

/// wait for a selector
#[cfg(feature = "chrome")]
pub async fn wait_for_selector(
    page: &chromiumoxide::Page,
    timeout: Option<core::time::Duration>,
    selector: &str,
) {
    let wait_until = async {
        loop {
            let sleep = tokio::time::sleep(tokio::time::Duration::from_millis(50));
            tokio::pin!(sleep);
            tokio::select! {
                _ = &mut sleep => (),
                v = page.find_element(selector) => {
                    if v.is_ok() {
                        break
                    }
                }
            }
        }
    };
    match timeout {
        Some(timeout) => if let Err(_) = tokio::time::timeout(timeout, wait_until).await {},
        _ => wait_until.await,
    }
}

/// Get the output path of a screenshot and create any parent folders if needed.
#[cfg(feature = "chrome")]
pub async fn create_output_path(
    base: &std::path::PathBuf,
    target_url: &str,
    format: &str,
) -> String {
    let out = string_concat!(
        &percent_encoding::percent_encode(
            target_url.as_bytes(),
            percent_encoding::NON_ALPHANUMERIC
        )
        .to_string(),
        format
    );

    let b = base.join(&out);

    match b.parent() {
        Some(p) => {
            let _ = tokio::fs::create_dir_all(&p).await;
        }
        _ => (),
    }

    b.display().to_string()
}

#[cfg(feature = "chrome")]
/// Wait for page events.
pub async fn page_wait(
    page: &chromiumoxide::Page,
    wait_for: &Option<crate::configuration::WaitFor>,
) {
    match wait_for {
        Some(wait_for) => {
            match wait_for.idle_network {
                Some(ref network_idle) => {
                    wait_for_event::<
                        chromiumoxide::cdp::browser_protocol::network::EventLoadingFinished,
                    >(page, network_idle.timeout)
                    .await;
                }
                _ => (),
            }

            match wait_for.selector {
                Some(ref await_for_selector) => {
                    wait_for_selector(
                        page,
                        await_for_selector.timeout,
                        &await_for_selector.selector,
                    )
                    .await;
                }
                _ => (),
            }

            match wait_for.delay {
                Some(ref wait_for_delay) => match wait_for_delay.timeout {
                    Some(timeout) => tokio::time::sleep(timeout).await,
                    _ => (),
                },
                _ => (),
            }
        }
        _ => (),
    }
}

#[derive(Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg(feature = "openai")]
/// The json response from OpenAI.
pub struct JsonResponse {
    /// The content returned.
    content: Vec<String>,
    /// The js script for the browser.
    js: String,
    #[cfg_attr(feature = "serde", serde(default))]
    /// The AI failed to parse the data.
    error: Option<String>,
}

/// Handle the OpenAI credits used. This does nothing without 'openai' feature flag.
#[cfg(feature = "openai")]
pub fn handle_openai_credits(
    page_response: &mut PageResponse,
    tokens_used: crate::features::openai_common::OpenAIUsage,
) {
    match page_response.openai_credits_used.as_mut() {
        Some(v) => v.push(tokens_used),
        None => page_response.openai_credits_used = Some(vec![tokens_used]),
    };
}

#[cfg(not(feature = "openai"))]
/// Handle the OpenAI credits used. This does nothing without 'openai' feature flag.
pub fn handle_openai_credits(
    _page_response: &mut PageResponse,
    _tokens_used: crate::features::openai_common::OpenAIUsage,
) {
}

/// Handle extra OpenAI data used. This does nothing without 'openai' feature flag.
#[cfg(feature = "openai")]
pub fn handle_extra_ai_data(
    page_response: &mut PageResponse,
    prompt: &str,
    x: JsonResponse,
    screenshot_output: Option<Vec<u8>>,
    error: Option<String>,
) {
    let ai_response = crate::page::AIResults {
        input: prompt.into(),
        js_output: x.js,
        content_output: x
            .content
            .iter()
            .map(|c| c.trim_start().into())
            .collect::<Vec<_>>(),
        screenshot_output,
        error,
    };

    match page_response.extra_ai_data.as_mut() {
        Some(v) => v.push(ai_response),
        None => page_response.extra_ai_data = Some(Vec::from([ai_response])),
    };
}

/// Extract to JsonResponse struct. This does nothing without 'openai' feature flag.
#[cfg(feature = "openai")]
pub fn handle_ai_data(js: &str) -> Option<JsonResponse> {
    match serde_json::from_str::<JsonResponse>(&js) {
        Ok(x) => Some(x),
        _ => None,
    }
}

#[cfg(feature = "chrome")]
#[derive(Default, Clone, Debug)]
/// The chrome HTTP response.
pub struct ChromeHTTPReqRes {
    /// Is the request blocked by a firewall?
    waf_check: bool,
    /// The HTTP status code.
    status_code: StatusCode,
    /// The HTTP method of the request.
    method: String,
    /// The HTTP response headers for the request.
    response_headers: std::collections::HashMap<String, String>,
    /// The HTTP request headers for the request.
    request_headers: std::collections::HashMap<String, String>,
    /// The HTTP protocol of the request.
    protocol: String,
}

#[cfg(feature = "chrome")]
/// Perform a http future with chrome.
pub async fn perform_chrome_http_request(
    page: &chromiumoxide::Page,
    source: &str,
) -> Result<ChromeHTTPReqRes, chromiumoxide::error::CdpError> {
    let mut waf_check = false;
    let mut status_code = StatusCode::OK;
    let mut method = String::from("GET");
    let mut response_headers = std::collections::HashMap::default();
    let mut request_headers = std::collections::HashMap::default();
    let mut protocol = String::from("http/1.1");

    match page
        .http_future(chromiumoxide::cdp::browser_protocol::page::NavigateParams {
            url: source.to_string(),
            transition_type: None,
            frame_id: None,
            referrer: None,
            referrer_policy: None,
        })?
        .await?
    {
        Some(http_request) => {
            match http_request.method.as_deref() {
                Some(http_method) => {
                    method = http_method.into();
                }
                _ => (),
            }

            request_headers.clone_from(&http_request.headers);

            match http_request.response {
                Some(ref response) => {
                    match response.protocol {
                        Some(ref p) => {
                            protocol.clone_from(p);
                        }
                        _ => (),
                    }

                    match response.headers.inner().as_object() {
                        Some(res_headers) => {
                            for (k, v) in res_headers {
                                response_headers.insert(k.to_string(), v.to_string());
                            }
                        }
                        _ => (),
                    }

                    if !response.url.starts_with(source) {
                        waf_check = match response.security_details {
                            Some(ref security_details) => {
                                if security_details.subject_name == "challenges.cloudflare.com" {
                                    true
                                } else {
                                    false
                                }
                            }
                            _ => response.url.contains("/cdn-cgi/challenge-platform"),
                        };
                        if !waf_check {
                            waf_check = match response.protocol {
                                Some(ref protocol) => protocol == "blob",
                                _ => false,
                            }
                        }
                    }

                    status_code = StatusCode::from_u16(response.status as u16)
                        .unwrap_or_else(|_| StatusCode::EXPECTATION_FAILED);
                }
                _ => (),
            }
        }
        _ => (),
    };

    Ok(ChromeHTTPReqRes {
        waf_check,
        status_code,
        method,
        response_headers,
        request_headers,
        protocol,
    })
}

/// Use OpenAI to extend the crawl. This does nothing without 'openai' feature flag.
#[cfg(all(feature = "chrome", not(feature = "openai")))]
pub async fn run_openai_request(
    _source: &str,
    _page: &chromiumoxide::Page,
    _wait_for: &Option<crate::configuration::WaitFor>,
    _openai_config: &Option<crate::configuration::GPTConfigs>,
    _page_response: &mut PageResponse,
    _ok: bool,
) {
}

/// Use OpenAI to extend the crawl. This does nothing without 'openai' feature flag.
#[cfg(all(feature = "chrome", feature = "openai"))]
pub async fn run_openai_request(
    source: &str,
    page: &chromiumoxide::Page,
    wait_for: &Option<crate::configuration::WaitFor>,
    openai_config: &Option<crate::configuration::GPTConfigs>,
    mut page_response: &mut PageResponse,
    ok: bool,
) {
    match &openai_config {
        Some(gpt_configs) => {
            let gpt_configs = match gpt_configs.prompt_url_map {
                Some(ref h) => {
                    let c = h.get::<case_insensitive_string::CaseInsensitiveString>(&source.into());

                    if !c.is_some() && gpt_configs.paths_map {
                        match url::Url::parse(source) {
                            Ok(u) => h.get::<case_insensitive_string::CaseInsensitiveString>(
                                &u.path().into(),
                            ),
                            _ => None,
                        }
                    } else {
                        c
                    }
                }
                _ => Some(gpt_configs),
            };

            match gpt_configs {
                Some(gpt_configs) => {
                    let mut prompts = gpt_configs.prompt.clone();

                    while let Some(prompt) = prompts.next() {
                        let gpt_results = if !gpt_configs.model.is_empty() && ok {
                            openai_request(
                                gpt_configs,
                                match page_response.content.as_ref() {
                                    Some(html) => String::from_utf8_lossy(html).to_string(),
                                    _ => Default::default(),
                                },
                                &source,
                                &prompt,
                            )
                            .await
                        } else {
                            Default::default()
                        };

                        let js_script = gpt_results.response;
                        let tokens_used = gpt_results.usage;
                        let gpt_error = gpt_results.error;

                        // set the credits used for the request
                        handle_openai_credits(&mut page_response, tokens_used);

                        let json_res = if gpt_configs.extra_ai_data {
                            match handle_ai_data(&js_script) {
                                Some(jr) => jr,
                                _ => {
                                    let mut jr = JsonResponse::default();
                                    jr.error = Some("An issue occured with serialization.".into());

                                    jr
                                }
                            }
                        } else {
                            let mut x = JsonResponse::default();
                            x.js = js_script;
                            x
                        };

                        // perform the js script on the page.
                        if !json_res.js.is_empty() {
                            let html: Option<bytes::Bytes> = match page
                                .evaluate_function(string_concat!(
                                    "async function() { ",
                                    json_res.js,
                                    "; return document.documentElement.outerHTML; }"
                                ))
                                .await
                            {
                                Ok(h) => match h.into_value() {
                                    Ok(hh) => Some(hh),
                                    _ => None,
                                },
                                _ => None,
                            };

                            if html.is_some() {
                                page_wait(&page, &wait_for).await;
                                if json_res.js.len() <= 400
                                    && json_res.js.contains("window.location")
                                {
                                    match page.content_bytes().await {
                                        Ok(b) => {
                                            page_response.content = Some(b);
                                        }
                                        _ => (),
                                    }
                                } else {
                                    page_response.content = html;
                                }
                            }
                        }

                        // attach the data to the page
                        if gpt_configs.extra_ai_data {
                            let screenshot_bytes = if gpt_configs.screenshot
                                && !json_res.js.is_empty()
                            {
                                let format = chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::Png;

                                let screenshot_configs =
                                    chromiumoxide::page::ScreenshotParams::builder()
                                        .format(format)
                                        .full_page(true)
                                        .quality(45)
                                        .omit_background(false);

                                match page.screenshot(screenshot_configs.build()).await {
                                    Ok(b) => {
                                        log::debug!("took screenshot: {:?}", source);
                                        Some(b)
                                    }
                                    Err(e) => {
                                        log::error!(
                                            "failed to take screenshot: {:?} - {:?}",
                                            e,
                                            source
                                        );
                                        None
                                    }
                                }
                            } else {
                                None
                            };

                            handle_extra_ai_data(
                                page_response,
                                &prompt,
                                json_res,
                                screenshot_bytes,
                                gpt_error,
                            );
                        }
                    }
                }
                _ => (),
            }
        }
        _ => (),
    };
}

/// Represents an HTTP version
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum HttpVersion {
    /// HTTP Version 0.9
    Http09,
    /// HTTP Version 1.0
    Http10,
    /// HTTP Version 1.1
    Http11,
    /// HTTP Version 2.0
    H2,
    /// HTTP Version 3.0
    H3,
}

/// A basic generic type that represents an HTTP response.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    /// HTTP response body
    pub body: Vec<u8>,
    /// HTTP response headers
    pub headers: std::collections::HashMap<String, String>,
    /// HTTP response status code
    pub status: u16,
    /// HTTP response url
    pub url: url::Url,
    /// HTTP response version
    pub version: HttpVersion,
}

/// A HTTP request type for caching.
#[cfg(feature = "cache_chrome_hybrid")]
pub struct HttpRequestLike {
    ///  The URI component of a request.
    pub uri: http::uri::Uri,
    /// The http method.
    pub method: reqwest::Method,
    /// The http headers.
    pub headers: http::HeaderMap,
}

#[cfg(feature = "cache_chrome_hybrid")]
/// A HTTP response type for caching.
pub struct HttpResponseLike {
    /// The http status code.
    pub status: StatusCode,
    /// The http headers.
    pub headers: http::HeaderMap,
}

#[cfg(feature = "cache_chrome_hybrid")]
impl RequestLike for HttpRequestLike {
    fn uri(&self) -> http::uri::Uri {
        self.uri.clone()
    }
    fn is_same_uri(&self, other: &http::Uri) -> bool {
        &self.uri == other
    }
    fn method(&self) -> &reqwest::Method {
        &self.method
    }
    fn headers(&self) -> &http::HeaderMap {
        &self.headers
    }
}

#[cfg(feature = "cache_chrome_hybrid")]
impl ResponseLike for HttpResponseLike {
    fn status(&self) -> StatusCode {
        self.status
    }
    fn headers(&self) -> &http::HeaderMap {
        &self.headers
    }
}

/// Convert headers to header map
#[cfg(feature = "cache_chrome_hybrid")]
pub fn convert_headers(headers: &std::collections::HashMap<String, String>) -> http::HeaderMap {
    let mut header_map = http::HeaderMap::new();

    for (index, items) in headers.iter().enumerate() {
        match http::HeaderValue::from_str(&items.1) {
            Ok(head) => {
                use std::str::FromStr;
                match http::HeaderName::from_str(&items.0) {
                    Ok(key) => {
                        header_map.insert(key, head);
                    }
                    _ => (),
                }
            }
            _ => (),
        }
        // mal headers
        if index > 2000 {
            break;
        }
    }

    header_map
}

#[cfg(feature = "cache_chrome_hybrid")]
/// Store the page to cache to be re-used across HTTP request.
pub async fn put_hybrid_cache(
    cache_key: &str,
    http_response: HttpResponse,
    method: &str,
    http_request_headers: std::collections::HashMap<String, String>,
) {
    use crate::http_cache_reqwest::CacheManager;
    use http_cache_semantics::CachePolicy;

    match http_response.url.as_str().parse::<http::uri::Uri>() {
        Ok(u) => {
            let req = HttpRequestLike {
                uri: u,
                method: reqwest::Method::from_bytes(method.as_bytes())
                    .unwrap_or(reqwest::Method::GET),
                headers: convert_headers(&http_response.headers),
            };

            let res = HttpResponseLike {
                status: StatusCode::from_u16(http_response.status)
                    .unwrap_or(StatusCode::EXPECTATION_FAILED),
                headers: convert_headers(&http_request_headers),
            };

            let policy = CachePolicy::new(&req, &res);

            let _ = crate::website::CACACHE_MANAGER
                .put(
                    cache_key.into(),
                    http_cache_reqwest::HttpResponse {
                        url: http_response.url,
                        body: http_response.body,
                        headers: http_response.headers,
                        version: match http_response.version {
                            HttpVersion::H2 => http_cache::HttpVersion::H2,
                            HttpVersion::Http10 => http_cache::HttpVersion::Http10,
                            HttpVersion::H3 => http_cache::HttpVersion::H3,
                            HttpVersion::Http09 => http_cache::HttpVersion::Http09,
                            HttpVersion::Http11 => http_cache::HttpVersion::Http11,
                        },
                        status: http_response.status,
                    },
                    policy,
                )
                .await;
        }
        _ => (),
    }
}

#[cfg(not(feature = "cache_chrome_hybrid"))]
/// Store the page to cache to be re-used across HTTP request.
pub async fn put_hybrid_cache(
    _cache_key: &str,
    _http_response: HttpResponse,
    _method: &str,
    _http_request_headers: std::collections::HashMap<String, String>,
) {
}

#[cfg(feature = "chrome")]
/// Perform a network request to a resource extracting all content as text streaming via chrome.
pub async fn fetch_page_html_chrome_base(
    source: &str,
    page: &chromiumoxide::Page,
    content: bool,
    wait_for_navigation: bool,
    wait_for: &Option<crate::configuration::WaitFor>,
    screenshot: &Option<crate::configuration::ScreenShotConfig>,
    page_set: bool,
    openai_config: &Option<crate::configuration::GPTConfigs>,
    url_target: Option<&str>,
) -> Result<PageResponse, chromiumoxide::error::CdpError> {
    let mut chrome_http_req_res = ChromeHTTPReqRes::default();

    let page = {
        // the active page was already set prior. No need to re-navigate or set the content.
        if !page_set {
            // used for smart mode re-rendering direct assigning html
            if content {
                page.set_content(source).await?
            } else {
                chrome_http_req_res = perform_chrome_http_request(&page, source).await?;

                page
            }
        } else {
            page
        }
    };

    // we do not need to wait for navigation if content is assigned. The method set_content already handles this.
    let final_url = if wait_for_navigation && !content {
        match page.wait_for_navigation_response().await {
            Ok(u) => get_last_redirect(&source, &u),
            _ => None,
        }
    } else {
        None
    };

    page_wait(&page, &wait_for).await;

    let mut res: bytes::Bytes = page.content_bytes().await?;

    if cfg!(feature = "real_browser") {
        let _ = cf_handle(&mut res, &page).await;
    };

    let ok = res.len() > 0;

    if chrome_http_req_res.waf_check && res.starts_with(b"<html><head>\n    <style global=") && res.ends_with(b";</script><iframe height=\"1\" width=\"1\" style=\"position: absolute; top: 0px; left: 0px; border: none; visibility: hidden;\"></iframe>\n\n</body></html>"){
        chrome_http_req_res.status_code = StatusCode::FORBIDDEN;
    }

    let mut page_response = PageResponse {
        content: if ok { Some(res) } else { None },
        status_code: chrome_http_req_res.status_code,
        final_url,
        ..Default::default()
    };

    run_openai_request(
        match url_target {
            Some(ref ut) => ut,
            _ => source,
        },
        page,
        wait_for,
        openai_config,
        &mut page_response,
        ok,
    )
    .await;

    if cfg!(feature = "chrome_screenshot") || screenshot.is_some() {
        perform_screenshot(source, page, screenshot, &mut page_response).await;
    }

    if !page_set && cfg!(feature = "cache_chrome_hybrid") {
        match url::Url::parse(source) {
            Ok(u) => {
                let http_response = HttpResponse {
                    url: u,
                    body: match page_response.content.as_ref() {
                        Some(b) => b.clone().to_vec(),
                        _ => Default::default(),
                    },
                    status: chrome_http_req_res.status_code.into(),
                    version: match chrome_http_req_res.protocol.as_str() {
                        "http/0.9" => HttpVersion::Http09,
                        "http/1" | "http/1.0" => HttpVersion::Http10,
                        "http/1.1" => HttpVersion::Http11,
                        "http/2.0" | "http/2" => HttpVersion::H2,
                        "http/3.0" | "http/3" => HttpVersion::H3,
                        _ => HttpVersion::Http11,
                    },
                    headers: chrome_http_req_res.response_headers,
                };
                put_hybrid_cache(
                    &string_concat!(chrome_http_req_res.method, ":", source),
                    http_response,
                    &chrome_http_req_res.method,
                    chrome_http_req_res.request_headers,
                )
                .await;
            }
            _ => (),
        }
    }

    if cfg!(not(feature = "chrome_store_page")) {
        page.execute(chromiumoxide::cdp::browser_protocol::page::CloseParams::default())
            .await?;
    }

    Ok(page_response)
}

/// Perform a screenshot shortcut.
#[cfg(feature = "chrome")]
pub async fn perform_screenshot(
    target_url: &str,
    page: &chromiumoxide::Page,
    screenshot: &Option<crate::configuration::ScreenShotConfig>,
    page_response: &mut PageResponse,
) {
    match screenshot {
        Some(ref ss) => {
            let output_format = string_concat!(
                ".",
                ss.params
                    .cdp_params
                    .format
                    .as_ref()
                    .unwrap_or_else(|| &crate::configuration::CaptureScreenshotFormat::Png)
                    .to_string()
            );
            let ss_params = chromiumoxide::page::ScreenshotParams::from(ss.params.clone());

            if ss.save {
                let output_path = create_output_path(
                    &ss.output_dir.clone().unwrap_or_else(|| "./storage/".into()),
                    &target_url,
                    &output_format,
                )
                .await;

                match page.save_screenshot(ss_params, &output_path).await {
                    Ok(b) => {
                        log::debug!("saved screenshot: {:?}", output_path);
                        if ss.bytes {
                            page_response.screenshot_bytes = Some(b);
                        }
                    }
                    Err(e) => {
                        log::error!("failed to save screenshot: {:?} - {:?}", e, output_path)
                    }
                };
            } else {
                match page.screenshot(ss_params).await {
                    Ok(b) => {
                        log::debug!("took screenshot: {:?}", target_url);
                        if ss.bytes {
                            page_response.screenshot_bytes = Some(b);
                        }
                    }
                    Err(e) => {
                        log::error!("failed to take screenshot: {:?} - {:?}", e, target_url)
                    }
                };
            }
        }
        _ => {
            let output_path = create_output_path(
                &std::env::var("SCREENSHOT_DIRECTORY")
                    .unwrap_or_else(|_| "./storage/".to_string())
                    .into(),
                &target_url,
                &".png",
            )
            .await;

            match page
                .save_screenshot(
                    chromiumoxide::page::ScreenshotParams::builder()
                        .format(
                            chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::Png,
                        )
                        .full_page(match std::env::var("SCREENSHOT_FULL_PAGE") {
                            Ok(t) => t == "true",
                            _ => true,
                        })
                        .omit_background(match std::env::var("SCREENSHOT_OMIT_BACKGROUND") {
                            Ok(t) => t == "true",
                            _ => true,
                        })
                        .build(),
                    &output_path,
                )
                .await
            {
                Ok(_) => log::debug!("saved screenshot: {:?}", output_path),
                Err(e) => log::error!("failed to save screenshot: {:?} - {:?}", e, output_path),
            };
        }
    }
}

#[cfg(feature = "chrome")]
/// Check if url matches the last item in a redirect chain for chrome CDP
pub fn get_last_redirect(
    target_url: &str,
    u: &Option<std::sync::Arc<chromiumoxide::handler::http::HttpRequest>>,
) -> Option<String> {
    match u {
        Some(u) => match u.redirect_chain.last()? {
            r => match r.url.as_ref()? {
                u => {
                    if target_url != u {
                        Some(u.into())
                    } else {
                        None
                    }
                }
            },
        },
        _ => None,
    }
}

/// Perform a network request to a resource extracting all content streaming.
pub async fn fetch_page_html_raw(target_url: &str, client: &Client) -> PageResponse {
    use crate::bytes::BufMut;
    use bytes::BytesMut;

    match client.get(target_url).send().await {
        Ok(res) if res.status().is_success() => {
            let u = res.url().as_str();

            let rd = if target_url != u {
                Some(u.into())
            } else {
                None
            };
            let status_code = res.status();
            #[cfg(feature = "headers")]
            let headers = res.headers().clone();
            let mut stream = res.bytes_stream();
            let mut data: BytesMut = BytesMut::new();

            while let Some(item) = stream.next().await {
                match item {
                    Ok(text) => {
                        let limit = *MAX_SIZE_BYTES;

                        if limit > 0 && data.len() + text.len() > limit {
                            break;
                        }

                        data.put(text)
                    }
                    _ => (),
                }
            }

            PageResponse {
                #[cfg(feature = "headers")]
                headers: Some(headers),
                content: Some(data.into()),
                final_url: rd,
                status_code,
                ..Default::default()
            }
        }
        Ok(res) => PageResponse {
            #[cfg(feature = "headers")]
            headers: Some(res.headers().clone()),
            status_code: res.status(),
            ..Default::default()
        },
        Err(_) => {
            log("- error parsing html text {}", target_url);
            Default::default()
        }
    }
}

#[cfg(all(not(feature = "fs"), not(feature = "chrome")))]
/// Perform a network request to a resource extracting all content as text streaming.
pub async fn fetch_page_html(target_url: &str, client: &Client) -> PageResponse {
    fetch_page_html_raw(target_url, client).await
}

/// Perform a network request to a resource extracting all content as text.
#[cfg(feature = "decentralized")]
pub async fn fetch_page(target_url: &str, client: &Client) -> Option<bytes::Bytes> {
    match client.get(target_url).send().await {
        Ok(res) if res.status().is_success() => match res.bytes().await {
            Ok(text) => Some(text),
            Err(_) => {
                log("- error fetching {}", &target_url);
                None
            }
        },
        Ok(_) => None,
        Err(_) => {
            log("- error parsing html bytes {}", &target_url);
            None
        }
    }
}

#[cfg(all(feature = "decentralized", feature = "headers"))]
/// Fetch a page with the headers returned.
pub enum FetchPageResult {
    /// Success extracting contents of the page
    Success(HeaderMap, Option<bytes::Bytes>),
    /// No success extracting content
    NoSuccess(HeaderMap),
    /// A network error occured.
    FetchError,
}

#[cfg(all(feature = "decentralized", feature = "headers"))]
/// Perform a network request to a resource with the response headers..
pub async fn fetch_page_and_headers(target_url: &str, client: &Client) -> FetchPageResult {
    match client.get(target_url).send().await {
        Ok(res) if res.status().is_success() => {
            let headers = res.headers().clone();
            let b = match res.bytes().await {
                Ok(text) => Some(text),
                Err(_) => {
                    log("- error fetching {}", &target_url);
                    None
                }
            };
            FetchPageResult::Success(headers, b)
        }
        Ok(res) => FetchPageResult::NoSuccess(res.headers().clone()),
        Err(_) => {
            log("- error parsing html bytes {}", &target_url);
            FetchPageResult::FetchError
        }
    }
}

/// Perform a network request to a resource extracting all content as text streaming.
#[cfg(feature = "fs")]
pub async fn fetch_page_html(target_url: &str, client: &Client) -> PageResponse {
    use crate::bytes::BufMut;
    use crate::tokio::io::AsyncReadExt;
    use crate::tokio::io::AsyncWriteExt;
    use bytes::BytesMut;
    use percent_encoding::utf8_percent_encode;
    use percent_encoding::NON_ALPHANUMERIC;
    use std::time::SystemTime;
    use tendril::fmt::Slice;

    lazy_static! {
        static ref TMP_DIR: String = {
            use std::fs;
            let mut tmp = std::env::temp_dir();

            tmp.push("spider/");

            // make sure spider dir is created.
            match fs::create_dir_all(&tmp) {
                Ok(_) => {
                    let dir_name = tmp.display().to_string();

                    match SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
                        Ok(dur) => {
                            string_concat!(dir_name, dur.as_secs().to_string())
                        }
                        _ => dir_name,
                    }
                }
                _ => "/tmp/".to_string()
            }
        };
    };

    match client.get(target_url).send().await {
        Ok(res) if res.status().is_success() => {
            let u = res.url().as_str();

            let rd = if target_url != u {
                Some(u.into())
            } else {
                None
            };

            let status_code = res.status();
            #[cfg(feature = "headers")]
            let headers = res.headers().clone();
            let mut stream = res.bytes_stream();
            let mut data: BytesMut = BytesMut::new();
            let mut file: Option<tokio::fs::File> = None;
            let mut file_path = String::new();

            while let Some(item) = stream.next().await {
                match item {
                    Ok(text) => {
                        let wrote_disk = file.is_some();

                        // perform operations entire in memory to build resource
                        if !wrote_disk && data.capacity() < 8192 {
                            data.put(text);
                        } else {
                            if !wrote_disk {
                                file_path = string_concat!(
                                    TMP_DIR,
                                    &utf8_percent_encode(target_url, NON_ALPHANUMERIC).to_string()
                                );
                                match tokio::fs::File::create(&file_path).await {
                                    Ok(f) => {
                                        let file = file.insert(f);

                                        data.put(text);

                                        match file.write_all(data.as_bytes()).await {
                                            Ok(_) => {
                                                data.clear();
                                            }
                                            _ => (),
                                        };
                                    }
                                    _ => data.put(text),
                                };
                            } else {
                                match &file.as_mut().unwrap().write_all(&text).await {
                                    Ok(_) => (),
                                    _ => data.put(text),
                                };
                            }
                        }
                    }
                    _ => (),
                }
            }

            PageResponse {
                #[cfg(feature = "headers")]
                headers: Some(headers),
                content: Some(if file.is_some() {
                    let mut buffer = vec![];

                    match tokio::fs::File::open(&file_path).await {
                        Ok(mut b) => match b.read_to_end(&mut buffer).await {
                            _ => (),
                        },
                        _ => (),
                    };

                    match tokio::fs::remove_file(file_path).await {
                        _ => (),
                    };

                    buffer.into()
                } else {
                    data.into()
                }),
                status_code,
                final_url: rd,
                ..Default::default()
            }
        }
        Ok(res) => PageResponse {
            #[cfg(feature = "headers")]
            headers: Some(res.headers().clone()),
            status_code: res.status(),
            ..Default::default()
        },
        Err(_) => {
            log("- error parsing html text {}", &target_url);
            Default::default()
        }
    }
}

#[cfg(all(not(feature = "fs"), feature = "chrome"))]
/// Perform a network request to a resource extracting all content as text streaming via chrome.
pub async fn fetch_page_html(
    target_url: &str,
    client: &Client,
    page: &chromiumoxide::Page,
    wait_for: &Option<crate::configuration::WaitFor>,
    screenshot: &Option<crate::configuration::ScreenShotConfig>,
    page_set: bool,
    openai_config: &Option<crate::configuration::GPTConfigs>,
) -> PageResponse {
    match fetch_page_html_chrome_base(
        &target_url,
        &page,
        false,
        true,
        wait_for,
        screenshot,
        page_set,
        openai_config,
        None,
    )
    .await
    {
        Ok(page) => page,
        Err(err) => {
            log::error!("{:?}", err);
            fetch_page_html_raw(&target_url, &client).await
        }
    }
}

#[cfg(feature = "chrome")]
/// Perform a network request to a resource extracting all content as text streaming via chrome.
pub async fn fetch_page_html_chrome(
    target_url: &str,
    client: &Client,
    page: &chromiumoxide::Page,
    wait_for: &Option<crate::configuration::WaitFor>,
    screenshot: &Option<crate::configuration::ScreenShotConfig>,
    page_set: bool,
    openai_config: &Option<crate::configuration::GPTConfigs>,
) -> PageResponse {
    match &page {
        page => {
            match fetch_page_html_chrome_base(
                &target_url,
                &page,
                false,
                true,
                wait_for,
                screenshot,
                page_set,
                openai_config,
                None,
            )
            .await
            {
                Ok(page) => page,
                _ => {
                    log(
                        "- error parsing html text defaulting to raw http request {}",
                        &target_url,
                    );

                    use crate::bytes::BufMut;
                    use bytes::BytesMut;

                    match client.get(target_url).send().await {
                        Ok(res) if res.status().is_success() => {
                            #[cfg(feature = "headers")]
                            let headers = res.headers().clone();
                            let status_code = res.status();
                            let mut stream = res.bytes_stream();
                            let mut data: BytesMut = BytesMut::new();

                            while let Some(item) = stream.next().await {
                                match item {
                                    Ok(text) => {
                                        let limit = *MAX_SIZE_BYTES;

                                        if limit > 0 && data.len() + text.len() > limit {
                                            break;
                                        }
                                        data.put(text)
                                    }
                                    _ => (),
                                }
                            }

                            PageResponse {
                                #[cfg(feature = "headers")]
                                headers: Some(headers),
                                content: Some(data.into()),
                                status_code,
                                ..Default::default()
                            }
                        }
                        Ok(res) => PageResponse {
                            #[cfg(feature = "headers")]
                            headers: Some(res.headers().clone()),
                            status_code: res.status(),
                            ..Default::default()
                        },
                        Err(_) => {
                            log("- error parsing html text {}", &target_url);
                            Default::default()
                        }
                    }
                }
            }
        }
    }
}

#[cfg(not(feature = "openai"))]
/// Perform a request to OpenAI Chat. This does nothing without the 'openai' flag enabled.
pub async fn openai_request(
    _gpt_configs: &crate::configuration::GPTConfigs,
    _resource: String,
    _url: &str,
    _prompt: &str,
) -> crate::features::openai_common::OpenAIReturn {
    Default::default()
}

#[cfg(feature = "openai")]
/// Perform a request to OpenAI Chat. This does nothing without the 'openai' flag enabled.
pub async fn openai_request_base(
    gpt_configs: &crate::configuration::GPTConfigs,
    resource: String,
    url: &str,
    prompt: &str,
) -> crate::features::openai_common::OpenAIReturn {
    lazy_static! {
        static ref CORE_BPE_TOKEN_COUNT: tiktoken_rs::CoreBPE = tiktoken_rs::cl100k_base().unwrap();
        static ref SEM: tokio::sync::Semaphore = {
            let logical = num_cpus::get();
            let physical = num_cpus::get_physical();

            let sem_limit = if logical > physical {
                (logical) / (physical)
            } else {
                logical
            };

            let (sem_limit, sem_max) = if logical == physical {
                (sem_limit * physical, 20)
            } else {
                (sem_limit * 4, 10)
            };
            let sem_limit = sem_limit / 3;
            tokio::sync::Semaphore::const_new(sem_limit.max(sem_max))
        };
        static ref CLIENT: async_openai::Client<async_openai::config::OpenAIConfig> =
            async_openai::Client::new();
    };

    match SEM.acquire().await {
        Ok(permit) => {
            let mut chat_completion_defaults =
                async_openai::types::CreateChatCompletionRequestArgs::default();
            let gpt_base = chat_completion_defaults
                .max_tokens(gpt_configs.max_tokens)
                .model(&gpt_configs.model);
            let gpt_base = match gpt_configs.user {
                Some(ref user) => gpt_base.user(user),
                _ => gpt_base,
            };
            let gpt_base = match gpt_configs.temperature {
                Some(temp) => gpt_base.temperature(temp),
                _ => gpt_base,
            };
            let gpt_base = match gpt_configs.top_p {
                Some(tp) => gpt_base.top_p(tp),
                _ => gpt_base,
            };

            let core_bpe = match tiktoken_rs::get_bpe_from_model(&gpt_configs.model) {
                Ok(bpe) => Some(bpe),
                _ => None,
            };

            let (tokens, prompt_tokens) = match core_bpe {
                Some(ref core_bpe) => (
                    core_bpe.encode_with_special_tokens(&resource),
                    core_bpe.encode_with_special_tokens(&prompt),
                ),
                _ => (
                    CORE_BPE_TOKEN_COUNT.encode_with_special_tokens(&resource),
                    CORE_BPE_TOKEN_COUNT.encode_with_special_tokens(&prompt),
                ),
            };

            // // we can use the output count later to perform concurrent actions.
            let output_tokens_count = tokens.len() + prompt_tokens.len();

            let max_tokens = crate::features::openai::calculate_max_tokens(
                &gpt_configs.model,
                gpt_configs.max_tokens,
                &&crate::features::openai::BROWSER_ACTIONS_SYSTEM_PROMPT_COMPLETION.clone(),
                &resource,
                &prompt,
            );

            // we need to slim down the content to fit the window.
            let resource = if output_tokens_count > max_tokens {
                let r = clean_html(&resource);

                let max_tokens = crate::features::openai::calculate_max_tokens(
                    &gpt_configs.model,
                    gpt_configs.max_tokens,
                    &&crate::features::openai::BROWSER_ACTIONS_SYSTEM_PROMPT_COMPLETION.clone(),
                    &r,
                    &prompt,
                );

                let (tokens, prompt_tokens) = match core_bpe {
                    Some(ref core_bpe) => (
                        core_bpe.encode_with_special_tokens(&r),
                        core_bpe.encode_with_special_tokens(&prompt),
                    ),
                    _ => (
                        CORE_BPE_TOKEN_COUNT.encode_with_special_tokens(&r),
                        CORE_BPE_TOKEN_COUNT.encode_with_special_tokens(&prompt),
                    ),
                };

                let output_tokens_count = tokens.len() + prompt_tokens.len();

                if output_tokens_count > max_tokens {
                    let r = clean_html_slim(&r);

                    let max_tokens = crate::features::openai::calculate_max_tokens(
                        &gpt_configs.model,
                        gpt_configs.max_tokens,
                        &&crate::features::openai::BROWSER_ACTIONS_SYSTEM_PROMPT_COMPLETION.clone(),
                        &r,
                        &prompt,
                    );

                    let (tokens, prompt_tokens) = match core_bpe {
                        Some(ref core_bpe) => (
                            core_bpe.encode_with_special_tokens(&r),
                            core_bpe.encode_with_special_tokens(&prompt),
                        ),
                        _ => (
                            CORE_BPE_TOKEN_COUNT.encode_with_special_tokens(&r),
                            CORE_BPE_TOKEN_COUNT.encode_with_special_tokens(&prompt),
                        ),
                    };

                    let output_tokens_count = tokens.len() + prompt_tokens.len();

                    if output_tokens_count > max_tokens {
                        clean_html_full(&r)
                    } else {
                        r
                    }
                } else {
                    r
                }
            } else {
                clean_html(&resource)
            };

            let mut tokens_used = crate::features::openai_common::OpenAIUsage::default();
            let json_mode = gpt_configs.extra_ai_data;

            match async_openai::types::ChatCompletionRequestAssistantMessageArgs::default()
                .content(&string_concat!("URL: ", url, "\n", "HTML: ", resource))
                .build()
            {
                Ok(resource_completion) => {
                    let mut messages: Vec<async_openai::types::ChatCompletionRequestMessage> =
                        vec![crate::features::openai::BROWSER_ACTIONS_SYSTEM_PROMPT.clone()];

                    if json_mode {
                        messages.push(
                            crate::features::openai::BROWSER_ACTIONS_SYSTEM_EXTRA_PROMPT.clone(),
                        );
                    }

                    messages.push(resource_completion.into());

                    if !prompt.is_empty() {
                        messages.push(
                            match async_openai::types::ChatCompletionRequestUserMessageArgs::default()
                            .content(prompt)
                            .build()
                        {
                            Ok(o) => o,
                            _ => Default::default(),
                        }
                        .into()
                        )
                    }

                    let v = match gpt_base
                        .max_tokens(max_tokens.max(1) as u16)
                        .messages(messages)
                        .response_format(async_openai::types::ChatCompletionResponseFormat {
                            r#type: if json_mode {
                                async_openai::types::ChatCompletionResponseFormatType::JsonObject
                            } else {
                                async_openai::types::ChatCompletionResponseFormatType::Text
                            },
                        })
                        .build()
                    {
                        Ok(request) => {
                            let res = match gpt_configs.api_key {
                                Some(ref key) => {
                                    if !key.is_empty() {
                                        let conf = CLIENT.config().to_owned();
                                        async_openai::Client::with_config(conf.with_api_key(key))
                                            .chat()
                                            .create(request)
                                            .await
                                    } else {
                                        CLIENT.chat().create(request).await
                                    }
                                }
                                _ => CLIENT.chat().create(request).await,
                            };

                            match res {
                                Ok(mut response) => {
                                    let mut choice = response.choices.first_mut();

                                    match response.usage.take() {
                                        Some(usage) => {
                                            tokens_used.prompt_tokens = usage.prompt_tokens;
                                            tokens_used.completion_tokens = usage.completion_tokens;
                                            tokens_used.total_tokens = usage.total_tokens;
                                        }
                                        _ => (),
                                    };

                                    match choice.as_mut() {
                                        Some(c) => match c.message.content.take() {
                                            Some(content) => content,
                                            _ => Default::default(),
                                        },
                                        _ => Default::default(),
                                    }
                                }
                                Err(err) => {
                                    log::error!("{:?}", err);
                                    Default::default()
                                }
                            }
                        }
                        _ => Default::default(),
                    };

                    drop(permit);

                    crate::features::openai_common::OpenAIReturn {
                        response: v,
                        usage: tokens_used,
                        error: None,
                    }
                }
                Err(e) => {
                    let mut d = crate::features::openai_common::OpenAIReturn::default();

                    d.error = Some(e.to_string());

                    d
                }
            }
        }
        Err(e) => {
            let mut d = crate::features::openai_common::OpenAIReturn::default();

            d.error = Some(e.to_string());

            d
        }
    }
}

#[cfg(all(feature = "openai", not(feature = "cache_openai")))]
/// Perform a request to OpenAI Chat. This does nothing without the 'openai' flag enabled.
pub async fn openai_request(
    gpt_configs: &crate::configuration::GPTConfigs,
    resource: String,
    url: &str,
    prompt: &str,
) -> crate::features::openai_common::OpenAIReturn {
    openai_request_base(gpt_configs, resource, url, prompt).await
}

#[cfg(all(feature = "openai", feature = "cache_openai"))]
/// Perform a request to OpenAI Chat. This does nothing without the 'openai' flag enabled.
pub async fn openai_request(
    gpt_configs: &crate::configuration::GPTConfigs,
    resource: String,
    url: &str,
    prompt: &str,
) -> crate::features::openai_common::OpenAIReturn {
    match &gpt_configs.cache {
        Some(cache) => {
            use std::hash::{DefaultHasher, Hash, Hasher};
            let mut s = DefaultHasher::new();

            url.hash(&mut s);
            prompt.hash(&mut s);
            gpt_configs.model.hash(&mut s);
            gpt_configs.max_tokens.hash(&mut s);
            gpt_configs.extra_ai_data.hash(&mut s);
            // non-determinstic
            resource.hash(&mut s);

            let key = s.finish();

            match cache.get(&key).await {
                Some(cache) => {
                    let mut c = cache;
                    c.usage.cached = true;
                    c
                }
                _ => {
                    let r = openai_request_base(gpt_configs, resource, url, prompt).await;
                    let _ = cache.insert(key, r.clone()).await;
                    r
                }
            }
        }
        _ => openai_request_base(gpt_configs, resource, url, prompt).await,
    }
}

/// Clean the html removing css and js default using the scraper crate.
pub fn clean_html_raw(html: &str) -> String {
    use crate::packages::scraper;
    lazy_static! {
        static ref SCRIPT_SELECTOR: scraper::Selector = scraper::Selector::parse("script").unwrap();
        static ref STYLE_SELECTOR: scraper::Selector = scraper::Selector::parse("style").unwrap();
    }
    let fragment = scraper::Html::parse_document(&html);
    let without_scripts: String = fragment
        .select(&SCRIPT_SELECTOR)
        .fold(html.to_string(), |acc, script| {
            acc.replace(&script.html(), "")
        });

    fragment
        .select(&STYLE_SELECTOR)
        .fold(without_scripts, |acc, style| acc.replace(&style.html(), ""))
}

/// Clean the html removing css and js
#[cfg(feature = "openai")]
pub fn clean_html_base(html: &str) -> String {
    use lol_html::{doc_comments, element, rewrite_str, RewriteStrSettings};

    match rewrite_str(
        html,
        RewriteStrSettings {
            element_content_handlers: vec![
                element!("script", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("style", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("link", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("iframe", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("[style*='display:none']", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("[id*='ad']", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("[class*='ad']", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("[id*='tracking']", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("[class*='tracking']", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("meta", |el| {
                    if let Some(attribute) = el.get_attribute("name") {
                        if attribute != "title" && attribute != "description" {
                            el.remove();
                        }
                    } else {
                        el.remove();
                    }
                    Ok(())
                }),
            ],
            document_content_handlers: vec![doc_comments!(|c| {
                c.remove();
                Ok(())
            })],
            ..RewriteStrSettings::default()
        },
    ) {
        Ok(r) => r,
        _ => clean_html_raw(html),
    }
}

/// Clean the HTML to slim fit GPT models. This removes base64 images from the prompt.
#[cfg(feature = "openai")]
pub fn clean_html_slim(html: &str) -> String {
    use lol_html::{doc_comments, element, rewrite_str, RewriteStrSettings};
    match rewrite_str(
        html,
        RewriteStrSettings {
            element_content_handlers: vec![
                element!("script", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("style", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("svg", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("noscript", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("link", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("iframe", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("canvas", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("video", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("img", |el| {
                    if let Some(src) = el.get_attribute("src") {
                        if src.starts_with("data:image") {
                            el.remove();
                        }
                    }
                    Ok(())
                }),
                element!("picture", |el| {
                    if let Some(src) = el.get_attribute("src") {
                        if src.starts_with("data:image") {
                            el.remove();
                        }
                    }
                    Ok(())
                }),
                element!("[style*='display:none']", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("[id*='ad']", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("[class*='ad']", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("[id*='tracking']", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("[class*='tracking']", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("meta", |el| {
                    if let Some(attribute) = el.get_attribute("name") {
                        if attribute != "title" && attribute != "description" {
                            el.remove();
                        }
                    } else {
                        el.remove();
                    }
                    Ok(())
                }),
            ],
            document_content_handlers: vec![doc_comments!(|c| {
                c.remove();
                Ok(())
            })],
            ..RewriteStrSettings::default()
        },
    ) {
        Ok(r) => r,
        _ => clean_html_raw(html),
    }
}

/// Clean the most of the extra properties in the html to fit the context.
#[cfg(feature = "openai")]
pub fn clean_html_full(html: &str) -> String {
    use lol_html::{doc_comments, element, rewrite_str, RewriteStrSettings};

    match rewrite_str(
        html,
        RewriteStrSettings {
            element_content_handlers: vec![
                element!("nav, footer", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("meta", |el| {
                    let name = el.get_attribute("name").map(|n| n.to_lowercase());

                    if !matches!(name.as_deref(), Some("viewport") | Some("charset")) {
                        el.remove();
                    }

                    Ok(())
                }),
                element!("*", |el| {
                    let attrs_to_keep = ["id", "data-", "class"];
                    let attributes_list = el.attributes().iter();
                    let mut remove_list = Vec::new();

                    for attr in attributes_list {
                        if !attrs_to_keep.contains(&attr.name().as_str()) {
                            remove_list.push(attr.name());
                        }
                    }

                    for attr in remove_list {
                        el.remove_attribute(&attr);
                    }

                    Ok(())
                }),
            ],
            document_content_handlers: vec![doc_comments!(|c| {
                c.remove();
                Ok(())
            })],
            ..RewriteStrSettings::default()
        },
    ) {
        Ok(r) => r,
        _ => clean_html_raw(html),
    }
}

/// Clean the html removing css and js
#[cfg(not(feature = "openai"))]
pub fn clean_html(html: &str) -> String {
    clean_html_raw(html)
}

/// Clean the html removing css and js
#[cfg(all(feature = "openai", not(feature = "openai_slim_fit")))]
pub fn clean_html(html: &str) -> String {
    clean_html_base(html)
}

/// Clean the html removing css and js
#[cfg(all(feature = "openai", feature = "openai_slim_fit"))]
pub fn clean_html(html: &str) -> String {
    clean_html_slim(html)
}

#[cfg(not(feature = "openai"))]
/// Clean and remove all base64 images from the prompt.
pub fn clean_html_slim(html: &str) -> String {
    html.into()
}

/// Log to console if configuration verbose.
pub fn log(message: &'static str, data: impl AsRef<str>) {
    if log_enabled!(Level::Info) {
        info!("{message} - {}", data.as_ref());
    }
}

#[cfg(feature = "control")]
/// determine action
#[derive(PartialEq, Debug)]
pub enum Handler {
    /// Crawl start state
    Start,
    /// Crawl pause state
    Pause,
    /// Crawl resume
    Resume,
    /// Crawl shutdown
    Shutdown,
}

#[cfg(feature = "control")]
lazy_static! {
    /// control handle for crawls
    pub static ref CONTROLLER: std::sync::Arc<tokio::sync::RwLock<(tokio::sync::watch::Sender<(String, Handler)>,
        tokio::sync::watch::Receiver<(String, Handler)>)>> =
            std::sync::Arc::new(tokio::sync::RwLock::new(tokio::sync::watch::channel(("handles".to_string(), Handler::Start))));
}

#[cfg(feature = "control")]
/// Pause a target website running crawl. The crawl_id is prepended directly to the domain and required if set. ex: d22323edsd-https://mydomain.com
pub async fn pause(target: &str) {
    match CONTROLLER
        .write()
        .await
        .0
        .send((target.into(), Handler::Pause))
    {
        _ => (),
    };
}

#[cfg(feature = "control")]
/// Resume a target website crawl. The crawl_id is prepended directly to the domain and required if set. ex: d22323edsd-https://mydomain.com
pub async fn resume(target: &str) {
    match CONTROLLER
        .write()
        .await
        .0
        .send((target.into(), Handler::Resume))
    {
        _ => (),
    };
}

#[cfg(feature = "control")]
/// Shutdown a target website crawl. The crawl_id is prepended directly to the domain and required if set. ex: d22323edsd-https://mydomain.com
pub async fn shutdown(target: &str) {
    match CONTROLLER
        .write()
        .await
        .0
        .send((target.into(), Handler::Shutdown))
    {
        _ => (),
    };
}

#[cfg(feature = "control")]
/// Reset a target website crawl. The crawl_id is prepended directly to the domain and required if set. ex: d22323edsd-https://mydomain.com
pub async fn reset(target: &str) {
    match CONTROLLER
        .write()
        .await
        .0
        .send((target.into(), Handler::Start))
    {
        _ => (),
    };
}
