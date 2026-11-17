use std::sync::LazyLock;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::Value;

use crate::networking::http_client::{HttpClient, Session};

pub struct BotGuardService {
    session: Session,
}

impl BotGuardService {
    const API_KEY: &str = "AIzaSyDyT5W0Jh49F30Pqqtyfdf7pDLFKLJoAnw";
    const REQUEST_KEY: &str = "O43z0dpjhgX20SCx4KAo";
    const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.3";

    pub fn shared() -> &'static BotGuardService {
        static SHARED: LazyLock<BotGuardService> =
            LazyLock::new(|| BotGuardService::new(Session::shared()));
        &SHARED
    }

    pub fn new(session: Session) -> Self {
        Self { session }
    }

    fn request(endpoint: &str, body: &[&str]) -> reqwest::Request {
        let url = format!("https://www.youtube.com/api/jnn/v1/{endpoint}");
        let mut request =
            reqwest::Request::new(reqwest::Method::POST, url.parse().expect("valid URL"));
        let headers = request.headers_mut();
        for (name, value) in [
            ("User-Agent", Self::USER_AGENT),
            ("Accept", "application/json"),
            ("Content-Type", "application/json+protobuf"),
            ("x-goog-api-key", Self::API_KEY),
            ("x-user-agent", "grpc-web-javascript/0.1"),
        ] {
            headers.insert(name, value.parse().expect("valid header"));
        }
        *request.body_mut() = Some(serde_json::to_vec(body).expect("strings encode").into());
        request
    }

    // Swift maps only `HttpError` to createFailed/generateITFailed and rethrows
    // `URLError`; Rust's `HttpError` covers both, so transport errors map too.

    pub async fn create_challenge(&self) -> Result<BotGuardChallenge, BotGuardError> {
        // Body: JSON array with requestKey.
        let response = HttpClient::validated_data(
            Self::request("Create", &[Self::REQUEST_KEY]),
            &self.session,
        )
        .await
        .map_err(|_| BotGuardError::CreateFailed)?;
        let body = String::from_utf8(response.data).map_err(|_| BotGuardError::InvalidResponse)?;
        Self::parse_challenge(&body)
    }

    /// (integrityToken, expiresIn).
    pub async fn generate_it(
        &self,
        botguard_response: &str,
    ) -> Result<(String, i64), BotGuardError> {
        // Body: JSON array [requestKey, botguardResponse].
        let request = Self::request("GenerateIT", &[Self::REQUEST_KEY, botguard_response]);
        let response = HttpClient::validated_data(request, &self.session)
            .await
            .map_err(|_| BotGuardError::GenerateITFailed)?;
        let body = String::from_utf8(response.data).map_err(|_| BotGuardError::InvalidResponse)?;
        Self::parse_integrity_token(&body)
    }

    // MARK: - Challenge Parsing

    /// Parses the raw Create response into challenge data for the JS engine.
    pub fn parse_challenge(raw: &str) -> Result<BotGuardChallenge, BotGuardError> {
        let json = decode_list(raw)
            .filter(|json| json.len() > 1)
            .ok_or(BotGuardError::InvalidResponse)?;

        // The challenge data is either at [0] or descrambled from [1].
        let challenge_array = if let Some(scrambled) = json[1].as_str() {
            decode_list(&Self::descramble(scrambled)).ok_or(BotGuardError::DescrambleFailed)?
        } else if let Some(array) = json[0].as_array() {
            array.clone()
        } else {
            return Err(BotGuardError::InvalidResponse);
        };

        if challenge_array.len() < 8 {
            return Err(BotGuardError::InvalidResponse);
        }
        let string = |index: usize| challenge_array[index].as_str().map(str::to_string);
        let (Some(message_id), Some(interpreter_hash), Some(program), Some(global_name)) =
            (string(0), string(3), string(4), string(5))
        else {
            return Err(BotGuardError::InvalidResponse);
        };

        // Extract interpreter JS from element [1] (may be nested).
        let interpreter_javascript = challenge_array[1]
            .as_array()
            .and_then(|scripts| scripts.iter().find_map(Value::as_str))
            .map(str::to_string);

        Ok(BotGuardChallenge {
            program,
            message_id: Some(message_id),
            interpreter_hash: Some(interpreter_hash),
            global_name: Some(global_name),
            interpreter_javascript,
        })
    }

    /// Parses the GenerateIT response (`[base64IntegrityToken, expirationSeconds]`).
    pub fn parse_integrity_token(raw: &str) -> Result<(String, i64), BotGuardError> {
        let json = decode_list(raw)
            .filter(|json| json.len() >= 2)
            .ok_or(BotGuardError::InvalidResponse)?;
        let (Some(token_b64), Some(expires_in)) = (json[0].as_str(), json[1].as_i64()) else {
            return Err(BotGuardError::InvalidResponse);
        };
        // Convert base64 to Uint8Array JS representation.
        Ok((Self::base64_to_u8(token_b64), expires_in))
    }

    /// Converts a base64 token to a JS Uint8Array string representation.
    pub fn base64_to_u8(base64: &str) -> String {
        let fixed = base64.replace('-', "+").replace('_', "/");
        let Ok(data) = STANDARD.decode(fixed) else {
            return "new Uint8Array([])".into();
        };
        let bytes: Vec<String> = data.iter().map(u8::to_string).collect();
        format!("new Uint8Array([{}])", bytes.join(","))
    }

    /// Descrambles challenge data: base64 decode + add 97 to each byte.
    pub fn descramble(scrambled: &str) -> String {
        let fixed = scrambled
            .replace('-', "+")
            .replace('_', "/")
            .replace('.', "=");
        let Ok(data) = STANDARD.decode(fixed) else {
            return scrambled.to_string();
        };
        let bytes: Vec<u8> = data.iter().map(|b| b.wrapping_add(97)).collect();
        String::from_utf8(bytes).unwrap_or_else(|_| scrambled.to_string())
    }
}

fn decode_list(raw: &str) -> Option<Vec<Value>> {
    match serde_json::from_str(raw) {
        Ok(Value::Array(list)) => Some(list),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BotGuardChallenge {
    pub program: String,
    pub message_id: Option<String>,
    pub interpreter_hash: Option<String>,
    pub global_name: Option<String>,
    pub interpreter_javascript: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BotGuardError {
    #[error("BotGuard Create API call failed")]
    CreateFailed,
    #[error("BotGuard GenerateIT API call failed")]
    GenerateITFailed,
    #[error("Invalid BotGuard API response")]
    InvalidResponse,
    #[error("Failed to descramble BotGuard program")]
    DescrambleFailed,
}
