#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// Wait for network request with optional timeout. This does nothing without the `chrome` flag enabled.
pub struct WaitForIdleNetwork {
    /// The max time to wait for the network. It is recommended to set this to a value around 30s. Set the value to None to remove the timeout.
    pub timeout: Option<core::time::Duration>,
}

impl WaitForIdleNetwork {
    /// Create new WaitForIdleNetwork with timeout.
    pub fn new(timeout: Option<core::time::Duration>) -> Self {
        Self { timeout }
    }
}

#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// Wait for a selector with optional timeout. This does nothing without the `chrome` flag enabled.
pub struct WaitForSelector {
    /// The max time to wait for the selector. It is recommended to set this to a value around 30s. Set the value to None to remove the timeout.
    pub timeout: Option<core::time::Duration>,
    /// The selector wait for
    pub selector: String,
}

impl WaitForSelector {
    /// Create new WaitForSelector with timeout.
    pub fn new(timeout: Option<core::time::Duration>, selector: String) -> Self {
        Self { timeout, selector }
    }
}

#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// Wait for with a delay. Should only be used for testing purposes. This does nothing without the `chrome` flag enabled.
pub struct WaitForDelay {
    /// The max time to wait. It is recommended to set this to a value around 30s. Set the value to None to remove the timeout.
    pub timeout: Option<core::time::Duration>,
}

impl WaitForDelay {
    /// Create new WaitForDelay with timeout.
    pub fn new(timeout: Option<core::time::Duration>) -> Self {
        Self { timeout }
    }
}

#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// The wait for options for the page. Multiple options can be set. This does nothing without the `chrome` flag enabled.
pub struct WaitFor {
    /// The max time to wait for the selector.
    pub selector: Option<WaitForSelector>,
    /// Wait for idle network 500ms.
    pub idle_network: Option<WaitForIdleNetwork>,
    /// Wait for delay. Should only be used for testing.
    pub delay: Option<WaitForDelay>,
    #[cfg_attr(feature = "serde", serde(default))]
    /// Wait for page navigations.
    pub page_navigations: bool,
}

impl WaitFor {
    /// Create new WaitFor with timeout.
    pub fn new(
        timeout: Option<core::time::Duration>,
        delay: Option<WaitForDelay>,
        page_navigations: bool,
        idle_network: bool,
        selector: Option<String>,
    ) -> Self {
        Self {
            page_navigations,
            idle_network: if idle_network {
                Some(WaitForIdleNetwork::new(timeout))
            } else {
                None
            },
            selector: if selector.is_some() {
                Some(WaitForSelector::new(timeout, selector.unwrap_or_default()))
            } else {
                None
            },
            delay,
        }
    }
}

#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Default, strum::EnumString, strum::Display, strum::AsRefStr,
)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// Capture screenshot options for chrome.
pub enum CaptureScreenshotFormat {
    #[cfg_attr(feature = "serde", serde(rename = "jpeg"))]
    /// jpeg format
    Jpeg,
    #[cfg_attr(feature = "serde", serde(rename = "png"))]
    #[default]
    /// png format
    Png,
    #[cfg_attr(feature = "serde", serde(rename = "webp"))]
    /// webp format
    Webp,
}

impl CaptureScreenshotFormat {
    /// convert the format to a lowercase string
    pub fn to_string(&self) -> String {
        self.as_ref().to_lowercase()
    }
}

#[cfg(feature = "chrome")]
impl From<CaptureScreenshotFormat>
    for chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat
{
    fn from(format: CaptureScreenshotFormat) -> Self {
        match format {
            CaptureScreenshotFormat::Jpeg => {
                chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::Jpeg
            }
            CaptureScreenshotFormat::Png => {
                chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::Png
            }
            CaptureScreenshotFormat::Webp => {
                chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::Webp
            }
        }
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// View port handling for chrome.
pub struct Viewport {
    /// Device screen Width
    pub width: u32,
    /// Device screen size
    pub height: u32,
    /// Device scale factor
    pub device_scale_factor: Option<f64>,
    /// Emulating Mobile?
    pub emulating_mobile: bool,
    /// Use landscape mode instead of portrait.
    pub is_landscape: bool,
    /// Touch screen device?
    pub has_touch: bool,
}

impl Default for Viewport {
    fn default() -> Self {
        Viewport {
            width: 800,
            height: 600,
            device_scale_factor: None,
            emulating_mobile: false,
            is_landscape: false,
            has_touch: false,
        }
    }
}

impl Viewport {
    /// Create a new viewport layout for chrome passing in the width.
    pub fn new(width: u32, height: u32) -> Self {
        Viewport {
            width,
            height,
            ..Default::default()
        }
    }
    /// Determine if the layout is a mobile device or not to emulate.
    pub fn set_mobile(&mut self, emulating_mobile: bool) {
        self.emulating_mobile = emulating_mobile;
    }
    /// Determine if the layout is in landscrape view or not to emulate.
    pub fn set_landscape(&mut self, is_landscape: bool) {
        self.is_landscape = is_landscape;
    }
    /// Determine if the device is a touch screen or not to emulate.
    pub fn set_touch(&mut self, has_touch: bool) {
        self.has_touch = has_touch;
    }
    /// The scale factor for the screen layout.
    pub fn set_scale_factor(&mut self, device_scale_factor: Option<f64>) {
        self.device_scale_factor = device_scale_factor;
    }
}

#[cfg(feature = "chrome")]
impl From<Viewport> for chromiumoxide::handler::viewport::Viewport {
    fn from(viewport: Viewport) -> Self {
        Self {
            width: viewport.width,
            height: viewport.height,
            device_scale_factor: viewport.device_scale_factor,
            emulating_mobile: viewport.emulating_mobile,
            is_landscape: viewport.is_landscape,
            has_touch: viewport.has_touch,
        }
    }
}

#[doc = "Capture page screenshot.\n[captureScreenshot](https://chromedevtools.github.io/devtools-protocol/tot/Page/#method-captureScreenshot)"]
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CaptureScreenshotParams {
    #[doc = "Image compression format (defaults to png)."]
    pub format: Option<CaptureScreenshotFormat>,
    #[doc = "Compression quality from range [0..100] (jpeg only)."]
    pub quality: Option<i64>,
    #[doc = "Capture the screenshot of a given region only."]
    pub clip: Option<ClipViewport>,
    #[doc = "Capture the screenshot from the surface, rather than the view. Defaults to true."]
    pub from_surface: Option<bool>,
    #[doc = "Capture the screenshot beyond the viewport. Defaults to false."]
    pub capture_beyond_viewport: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// The view port clip for screenshots.
pub struct ClipViewport {
    #[doc = "X offset in device independent pixels (dip)."]
    #[cfg_attr(feature = "serde", serde(rename = "x"))]
    pub x: f64,
    #[doc = "Y offset in device independent pixels (dip)."]
    #[cfg_attr(feature = "serde", serde(rename = "y"))]
    pub y: f64,
    #[doc = "Rectangle width in device independent pixels (dip)."]
    #[cfg_attr(feature = "serde", serde(rename = "width"))]
    pub width: f64,
    #[doc = "Rectangle height in device independent pixels (dip)."]
    #[cfg_attr(feature = "serde", serde(rename = "height"))]
    pub height: f64,
    #[doc = "Page scale factor."]
    #[cfg_attr(feature = "serde", serde(rename = "scale"))]
    pub scale: f64,
}

#[cfg(feature = "chrome")]
impl From<ClipViewport> for chromiumoxide::cdp::browser_protocol::page::Viewport {
    fn from(viewport: ClipViewport) -> Self {
        Self {
            x: viewport.x,
            y: viewport.y,
            height: viewport.height,
            width: viewport.width,
            scale: viewport.scale,
        }
    }
}

/// Screenshot configuration.
#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ScreenShotConfig {
    /// The screenshot params.
    pub params: ScreenshotParams,
    /// Return the bytes of the screenshot on the Page.
    pub bytes: bool,
    /// Store the screenshot to disk. This can be used with output_dir. If disabled will not store the file to the output directory.
    pub save: bool,
    /// The output directory to store the file. Parant folders may be created inside the directory.
    pub output_dir: Option<std::path::PathBuf>,
}

impl ScreenShotConfig {
    /// Create a new screenshot configuration.
    pub fn new(
        params: ScreenshotParams,
        bytes: bool,
        save: bool,
        output_dir: Option<std::path::PathBuf>,
    ) -> Self {
        Self {
            params,
            bytes,
            save,
            output_dir,
        }
    }
}

/// The screenshot params for the page.
#[derive(Default, Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ScreenshotParams {
    /// Chrome DevTools Protocol screenshot options.
    pub cdp_params: CaptureScreenshotParams,
    /// Take full page screenshot.
    pub full_page: Option<bool>,
    /// Make the background transparent (png only).
    pub omit_background: Option<bool>,
}

impl ScreenshotParams {
    /// Create a new ScreenshotParams.
    pub fn new(
        cdp_params: CaptureScreenshotParams,
        full_page: Option<bool>,
        omit_background: Option<bool>,
    ) -> Self {
        Self {
            cdp_params,
            full_page,
            omit_background,
        }
    }
}

#[cfg(feature = "chrome")]
impl From<ScreenshotParams> for chromiumoxide::page::ScreenshotParams {
    fn from(params: ScreenshotParams) -> Self {
        let full_page = if params.full_page.is_some() {
            match params.full_page {
                Some(v) => v,
                _ => false,
            }
        } else {
            match std::env::var("SCREENSHOT_FULL_PAGE") {
                Ok(t) => t == "true",
                _ => true,
            }
        };
        let omit_background = if params.omit_background.is_some() {
            match params.omit_background {
                Some(v) => v,
                _ => false,
            }
        } else {
            match std::env::var("SCREENSHOT_OMIT_BACKGROUND") {
                Ok(t) => t == "true",
                _ => true,
            }
        };
        let format = if params.cdp_params.format.is_some() {
            match params.cdp_params.format {
                Some(v) => v.into(),
                _ => chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::Png,
            }
        } else {
            chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::Png
        };

        let params_builder = chromiumoxide::page::ScreenshotParams::builder()
            .format(format)
            .full_page(full_page)
            .omit_background(omit_background);

        let params_builder = if params.cdp_params.quality.is_some() {
            params_builder.quality(params.cdp_params.quality.unwrap_or(75))
        } else {
            params_builder
        };

        let params_builder = if params.cdp_params.clip.is_some() {
            match params.cdp_params.clip {
                Some(vp) => params_builder.clip(
                    chromiumoxide::cdp::browser_protocol::page::Viewport::from(vp),
                ),
                _ => params_builder,
            }
        } else {
            params_builder
        };

        let params_builder = if params.cdp_params.capture_beyond_viewport.is_some() {
            match params.cdp_params.capture_beyond_viewport {
                Some(capture_beyond_viewport) => {
                    params_builder.capture_beyond_viewport(capture_beyond_viewport)
                }
                _ => params_builder,
            }
        } else {
            params_builder
        };

        let params_builder = if params.cdp_params.from_surface.is_some() {
            match params.cdp_params.from_surface {
                Some(from_surface) => params_builder.from_surface(from_surface),
                _ => params_builder,
            }
        } else {
            params_builder
        };

        params_builder.build()
    }
}

#[doc = "The decision on what to do in response to the authorization challenge.  Default means\ndeferring to the default behavior of the net stack, which will likely either the Cancel\nauthentication or display a popup dialog box."]
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AuthChallengeResponseResponse {
    #[default]
    /// The default.
    Default,
    /// Cancel the authentication prompt.
    CancelAuth,
    /// Provide credentials.
    ProvideCredentials,
}

#[doc = "Response to an AuthChallenge.\n[AuthChallengeResponse](https://chromedevtools.github.io/devtools-protocol/tot/Fetch/#type-AuthChallengeResponse)"]
#[derive(Default, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AuthChallengeResponse {
    #[doc = "The decision on what to do in response to the authorization challenge.  Default means\ndeferring to the default behavior of the net stack, which will likely either the Cancel\nauthentication or display a popup dialog box."]
    pub response: AuthChallengeResponseResponse,
    #[doc = "The username to provide, possibly empty. Should only be set if response is\nProvideCredentials."]
    pub username: Option<String>,
    #[doc = "The password to provide, possibly empty. Should only be set if response is\nProvideCredentials."]
    pub password: Option<String>,
}

#[cfg(feature = "chrome")]
impl From<AuthChallengeResponse>
    for chromiumoxide::cdp::browser_protocol::fetch::AuthChallengeResponse
{
    fn from(auth_challenge_response: AuthChallengeResponse) -> Self {
        Self {
            response: match auth_challenge_response.response {
                AuthChallengeResponseResponse::CancelAuth => chromiumoxide::cdp::browser_protocol::fetch::AuthChallengeResponseResponse::CancelAuth ,
                AuthChallengeResponseResponse::ProvideCredentials => chromiumoxide::cdp::browser_protocol::fetch::AuthChallengeResponseResponse::ProvideCredentials ,
                AuthChallengeResponseResponse::Default => chromiumoxide::cdp::browser_protocol::fetch::AuthChallengeResponseResponse::Default ,

            },
            username: auth_challenge_response.username,
            password: auth_challenge_response.password
        }
    }
}
