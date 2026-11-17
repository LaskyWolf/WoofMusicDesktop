/// Client identity per request. Swift `static let` constants are SCREAMING_SNAKE
/// associated consts here (`androidVr1_65_10` → `ANDROID_VR_1_65_10`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct YouTubeClient {
    pub client_name: &'static str,
    pub client_version: &'static str,
    pub client_id: i64,
    pub user_agent: &'static str,
    pub login_supported: bool,
    pub use_music_player_endpoint: bool,
    pub is_embedded: bool,
    pub include_user_agent_in_context: bool,
    pub use_signature_timestamp: bool,
    pub use_web_po_tokens: bool,
    pub skip_player_response_validation: bool,
    pub os_name: Option<&'static str>,
    pub os_version: Option<&'static str>,
    pub device_make: Option<&'static str>,
    pub device_model: Option<&'static str>,
    pub android_sdk_version: Option<&'static str>,
    pub platform: Option<&'static str>,
    pub client_screen: Option<&'static str>,
    pub embed_url: Option<&'static str>,
}

const TV_HTML5_USER_AGENT: &str = "Mozilla/5.0 (ChromiumStylePlatform) Cobalt/25.lts.30.1034943-gold (unlike Gecko), Unknown_TV_Unknown_0/Unknown (Unknown, Unknown)";

impl YouTubeClient {
    /// Swift init defaults: every flag false, every optional nil.
    pub const fn new(
        client_name: &'static str,
        client_version: &'static str,
        client_id: i64,
        user_agent: &'static str,
    ) -> Self {
        Self {
            client_name,
            client_version,
            client_id,
            user_agent,
            login_supported: false,
            use_music_player_endpoint: false,
            is_embedded: false,
            include_user_agent_in_context: false,
            use_signature_timestamp: false,
            use_web_po_tokens: false,
            skip_player_response_validation: false,
            os_name: None,
            os_version: None,
            device_make: None,
            device_model: None,
            android_sdk_version: None,
            platform: None,
            client_screen: None,
            embed_url: None,
        }
    }

    pub const WEB_USER_AGENT: &str =
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:140.0) Gecko/20100101 Firefox/140.0";

    /// YouTube Music default.
    pub const WEB_REMIX: Self = Self {
        login_supported: true,
        use_signature_timestamp: true,
        use_web_po_tokens: true,
        ..Self::new("WEB_REMIX", "1.20260707.12.00", 67, Self::WEB_USER_AGENT)
    };

    /// Main YouTube web default.
    pub const WEB: Self = Self {
        login_supported: true,
        include_user_agent_in_context: true,
        use_signature_timestamp: true,
        use_web_po_tokens: true,
        os_name: Some("Windows"),
        os_version: Some("10.0"),
        platform: Some("DESKTOP"),
        ..Self::new("WEB", "2.20261002.01.00", 1, Self::WEB_USER_AGENT)
    };

    /// Lightweight duration / metadata fetch, no cipher needed.
    pub const IOS: Self = Self {
        include_user_agent_in_context: true,
        os_name: Some("iPhone"),
        os_version: Some("18.3.2.22D82"),
        device_make: Some("Apple"),
        device_model: Some("iPhone16,2"),
        ..Self::new(
            "IOS",
            "21.26.4",
            5,
            "com.google.ios.youtube/21.26.4 (iPhone16,2; U; CPU iOS 18_3_2 like Mac OS X;)",
        )
    };

    /// yt-dlp's default Android VR profile - stable direct URLs.
    pub const ANDROID_VR_1_65_10: Self = Self {
        use_music_player_endpoint: true,
        include_user_agent_in_context: true,
        os_name: Some("Android"),
        os_version: Some("12L"),
        device_make: Some("Oculus"),
        device_model: Some("Quest 3"),
        android_sdk_version: Some("32"),
        ..Self::new(
            "ANDROID_VR",
            "1.65.10",
            28,
            "com.google.android.apps.youtube.vr.oculus/1.65.10 (Linux; U; Android 12L; eureka-user Build/SQ3A.220605.009.A1) gzip",
        )
    };

    pub const ANDROID_VR_1_43_32: Self = Self {
        use_music_player_endpoint: true,
        include_user_agent_in_context: true,
        os_name: Some("Android"),
        os_version: Some("12"),
        device_make: Some("Oculus"),
        device_model: Some("Quest 3"),
        android_sdk_version: Some("32"),
        ..Self::new(
            "ANDROID_VR",
            "1.43.32",
            28,
            "com.google.android.apps.youtube.vr.oculus/1.43.32 (Linux; U; Android 12; en_US; Quest 3; Build/SQ3A.220605.009.A1; Cronet/107.0.5284.2)",
        )
    };

    /// Newer Android VR version - direct URLs.
    pub const ANDROID_VR_1_61_48: Self = Self {
        use_music_player_endpoint: true,
        include_user_agent_in_context: true,
        os_name: Some("Android"),
        os_version: Some("12"),
        device_make: Some("Oculus"),
        device_model: Some("Quest 3"),
        android_sdk_version: Some("32"),
        ..Self::new(
            "ANDROID_VR",
            "1.61.48",
            28,
            "com.google.android.apps.youtube.vr.oculus/1.61.48 (Linux; U; Android 12; en_US; Quest 3; Build/SQ3A.220605.009.A1; Cronet/132.0.6808.3)",
        )
    };

    pub const TV_HTML5: Self = Self {
        login_supported: true,
        include_user_agent_in_context: true,
        use_signature_timestamp: true,
        use_web_po_tokens: true,
        ..Self::new("TVHTML5", "7.20260707.07.00", 7, TV_HTML5_USER_AGENT)
    };

    pub const VISION_OS: Self = Self {
        use_music_player_endpoint: true,
        skip_player_response_validation: true,
        os_name: Some("VISION_OS"),
        os_version: Some("1.3"),
        device_make: Some("Apple"),
        device_model: Some("RealityDevice14,1"),
        platform: Some("MOBILE"),
        ..Self::new(
            "VISIONOS",
            "0.1",
            101,
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_6) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Safari/605.1.15",
        )
    };

    /// Late fallback; needs platform attestation.
    pub const MOBILE: Self = Self {
        include_user_agent_in_context: true,
        os_name: Some("Android"),
        os_version: Some("11"),
        android_sdk_version: Some("30"),
        ..Self::new(
            "ANDROID",
            "21.26.364",
            3,
            "com.google.android.youtube/21.26.364 (Linux; U; Android 11) gzip",
        )
    };

    /// Embedded player; bypasses age restriction without login.
    pub const TV_HTML5_SIMPLY_EMBEDDED: Self = Self {
        is_embedded: true,
        use_signature_timestamp: true,
        use_web_po_tokens: true,
        embed_url: Some("https://www.reddit.com/"),
        ..Self::new(
            "TVHTML5_SIMPLY_EMBEDDED_PLAYER",
            "2.0",
            85,
            TV_HTML5_USER_AGENT,
        )
    };
}
