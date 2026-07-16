use base64::{engine::general_purpose::STANDARD, Engine as _};
use futures_util::StreamExt;
use reqwest::{
    header::{HeaderMap, HeaderValue},
    Client,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::audio::PlaybackHandle;

const TTS_ENDPOINT: &str = "https://openspeech.bytedance.com/api/v3/tts/unidirectional";
const MAX_PENDING_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_SEGMENT_CHARS: usize = 450;
const MIN_SEGMENT_CHARS: usize = 120;
const RESPONSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
const STREAM_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Clone)]
pub struct TtsOptions {
    pub api_key: String,
    pub resource_id: String,
    pub speaker: String,
    pub speech_rate: i32,
    pub loudness_rate: i32,
    pub sample_rate: u32,
}

#[derive(Clone)]
pub struct TtsClient {
    client: Client,
}

impl Default for TtsClient {
    fn default() -> Self {
        Self {
            client: Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("the embedded HTTP client configuration is valid"),
        }
    }
}

impl TtsClient {
    pub async fn synthesize_into(
        &self,
        text: &str,
        options: &TtsOptions,
        playback: &PlaybackHandle,
    ) -> Result<(), String> {
        for segment in split_text(text) {
            self.synthesize_segment_into(&segment, options, playback)
                .await?;
        }
        Ok(())
    }

    async fn synthesize_segment_into(
        &self,
        text: &str,
        options: &TtsOptions,
        playback: &PlaybackHandle,
    ) -> Result<(), String> {
        let request_id = Uuid::new_v4().to_string();
        let request = self
            .client
            .post(TTS_ENDPOINT)
            .headers(headers(
                &options.api_key,
                &options.resource_id,
                &request_id,
            )?)
            .json(&request_body(text, options))
            .send();
        let response = tokio::select! {
            _ = playback.cancelled() => return Ok(()),
            response = tokio::time::timeout(RESPONSE_TIMEOUT, request) => response
                .map_err(|_| "等待火山引擎语音响应超时。".to_owned())?
                .map_err(|error| format!("无法连接火山引擎语音服务：{error}"))?,
        };

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let error = response.json::<TtsStreamResponse>().await.ok();
            return Err(http_error(status, error.as_ref()));
        }

        let mut chunks = response.bytes_stream();
        let mut parser = TtsStreamParser::default();
        let mut received_audio = 0_usize;
        loop {
            let next = tokio::select! {
                _ = playback.cancelled() => return Ok(()),
                next = tokio::time::timeout(STREAM_IDLE_TIMEOUT, chunks.next()) => next
                    .map_err(|_| "接收语音数据超时，请检查网络后重试。".to_owned())?,
            };
            match next {
                Some(Ok(chunk)) => {
                    for event in parser.push(&chunk)? {
                        match event {
                            TtsStreamEvent::Audio(audio) => {
                                received_audio += audio.len();
                                playback.push_pcm(&audio).await;
                            }
                            TtsStreamEvent::Finished => return audio_result(received_audio),
                        }
                    }
                }
                Some(Err(error)) => return Err(format!("接收语音数据时中断：{error}")),
                None => {
                    for event in parser.finish()? {
                        match event {
                            TtsStreamEvent::Audio(audio) => {
                                received_audio += audio.len();
                                playback.push_pcm(&audio).await;
                            }
                            TtsStreamEvent::Finished => return audio_result(received_audio),
                        }
                    }
                    return audio_result(received_audio);
                }
            }
        }
    }
}

fn split_text(text: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut current_chars = 0;

    for character in text.chars() {
        current.push(character);
        current_chars += 1;
        let at_natural_break = current_chars >= MIN_SEGMENT_CHARS
            && matches!(character, '。' | '！' | '？' | '；' | '!' | '?' | ';');
        if current_chars >= MAX_SEGMENT_CHARS || at_natural_break {
            push_segment(&mut segments, &mut current);
            current_chars = 0;
        }
    }
    push_segment(&mut segments, &mut current);
    segments
}

fn push_segment(segments: &mut Vec<String>, current: &mut String) {
    let segment = current.trim();
    if !segment.is_empty() {
        segments.push(segment.to_owned());
    }
    current.clear();
}

fn headers(api_key: &str, resource_id: &str, request_id: &str) -> Result<HeaderMap, String> {
    let api_key =
        HeaderValue::from_str(api_key.trim()).map_err(|_| "API Key 格式无效。".to_owned())?;
    let resource_id =
        HeaderValue::from_str(resource_id.trim()).map_err(|_| "音色资源标识无效。".to_owned())?;
    let request_id =
        HeaderValue::from_str(request_id).map_err(|_| "无法生成有效的请求标识。".to_owned())?;

    let mut headers = HeaderMap::new();
    headers.insert("X-Api-Key", api_key);
    headers.insert("X-Api-Resource-Id", resource_id);
    headers.insert("X-Api-Request-Id", request_id);
    Ok(headers)
}

fn request_body<'a>(text: &'a str, options: &'a TtsOptions) -> TtsRequest<'a> {
    TtsRequest {
        user: User { uid: "xuandu-desktop" },
        req_params: RequestParams {
            text,
            speaker: &options.speaker,
            audio_params: AudioParams {
                format: "pcm",
                sample_rate: options.sample_rate,
                speech_rate: options.speech_rate,
                loudness_rate: options.loudness_rate,
            },
            additions: r#"{"explicit_language":"zh-cn","disable_markdown_filter":true,"disable_emoji_filter":true}"#.to_owned(),
        },
    }
}

#[derive(Serialize)]
struct TtsRequest<'a> {
    user: User,
    req_params: RequestParams<'a>,
}

#[derive(Serialize)]
struct User {
    uid: &'static str,
}

#[derive(Serialize)]
struct RequestParams<'a> {
    text: &'a str,
    speaker: &'a str,
    audio_params: AudioParams,
    additions: String,
}

#[derive(Serialize)]
struct AudioParams {
    format: &'static str,
    sample_rate: u32,
    speech_rate: i32,
    loudness_rate: i32,
}

#[derive(Debug, Deserialize)]
struct TtsStreamResponse {
    #[serde(default)]
    code: Option<i64>,
    #[serde(default)]
    data: Option<String>,
    #[serde(default)]
    header: Option<TtsErrorHeader>,
}

impl TtsStreamResponse {
    fn status_code(&self) -> Option<i64> {
        self.code
            .or_else(|| self.header.as_ref().and_then(|header| header.code))
    }
}

#[derive(Debug, Deserialize)]
struct TtsErrorHeader {
    #[serde(default)]
    code: Option<i64>,
}

#[derive(Debug, PartialEq, Eq)]
enum TtsStreamEvent {
    Audio(Vec<u8>),
    Finished,
}

#[derive(Default)]
struct TtsStreamParser {
    pending: Vec<u8>,
}

impl TtsStreamParser {
    fn push(&mut self, chunk: &[u8]) -> Result<Vec<TtsStreamEvent>, String> {
        self.pending.extend_from_slice(chunk);
        if self.pending.len() > MAX_PENDING_RESPONSE_BYTES {
            return Err("火山引擎返回了无法解析的过长响应。".to_owned());
        }

        let (consumed, events) = {
            let mut stream = serde_json::Deserializer::from_slice(&self.pending)
                .into_iter::<TtsStreamResponse>();
            let mut events = Vec::new();
            while let Some(response) = stream.next() {
                match response {
                    Ok(response) => {
                        if let Some(event) = response.into_event()? {
                            events.push(event);
                        }
                    }
                    Err(error) if error.is_eof() => break,
                    Err(_) => return Err("无法解析火山引擎的语音响应。".to_owned()),
                }
            }
            (stream.byte_offset(), events)
        };
        if consumed > 0 {
            self.pending.drain(..consumed);
        }
        Ok(events)
    }

    fn finish(&mut self) -> Result<Vec<TtsStreamEvent>, String> {
        let events = self.push(&[])?;
        if self.pending.iter().any(|byte| !byte.is_ascii_whitespace()) {
            return Err("火山引擎语音响应不完整。".to_owned());
        }
        Ok(events)
    }
}

impl TtsStreamResponse {
    fn into_event(self) -> Result<Option<TtsStreamEvent>, String> {
        match self.status_code().unwrap_or(0) {
            0 => match self.data.filter(|data| !data.is_empty()) {
                Some(data) => STANDARD
                    .decode(data)
                    .map(TtsStreamEvent::Audio)
                    .map(Some)
                    .map_err(|_| "火山引擎返回了无效的音频数据。".to_owned()),
                None => Ok(None),
            },
            20_000_000 => Ok(Some(TtsStreamEvent::Finished)),
            code => Err(provider_error(code)),
        }
    }
}

fn audio_result(received_audio: usize) -> Result<(), String> {
    if received_audio == 0 {
        Err("火山引擎未返回可播放的音频。请检查音色权限和服务开通状态。".to_owned())
    } else {
        Ok(())
    }
}

fn http_error(status: u16, response: Option<&TtsStreamResponse>) -> String {
    let detail = response
        .and_then(TtsStreamResponse::status_code)
        .map(provider_error)
        .unwrap_or_else(|| "火山引擎请求未成功。".to_owned());
    match status {
        401 | 403 => {
            format!("火山引擎鉴权失败（HTTP {status}）。请确认使用的是豆包语音控制台 API Key。")
        }
        429 => "火山引擎请求受限（HTTP 429）。请检查额度或稍后重试。".to_owned(),
        _ => format!("火山引擎请求失败（HTTP {status}）。{detail}"),
    }
}

fn provider_error(code: i64) -> String {
    match code {
        45_000_010 => "火山引擎 API Key 无效。请确认使用的是豆包语音控制台 API Key。".to_owned(),
        45_000_000 => "所选音色不可用。请检查音色权限。".to_owned(),
        55_000_000 => "所选音色与 seed-tts-2.0 不匹配。请选择 2.0 音色。".to_owned(),
        40_402_003 => "本次文本超过语音服务限制。请缩短文本。".to_owned(),
        _ => format!("火山引擎语音合成失败（代码 {code}）。"),
    }
}

#[cfg(test)]
mod tests {
    use super::{request_body, split_text, TtsOptions, TtsStreamEvent, TtsStreamParser};

    #[test]
    fn request_uses_v3_pcm_layout_and_never_serializes_the_api_key() {
        let options = TtsOptions {
            api_key: "secret-key".to_owned(),
            resource_id: "seed-tts-2.0".to_owned(),
            speaker: "speaker-id".to_owned(),
            speech_rate: 0,
            loudness_rate: 0,
            sample_rate: 48_000,
        };
        let body = serde_json::to_value(request_body("你好", &options)).unwrap();

        assert_eq!(body["user"]["uid"], "xuandu-desktop");
        assert_eq!(body["req_params"]["audio_params"]["format"], "pcm");
        assert!(body["audio_params"].is_null());
        assert!(body["req_params"]["additions"]
            .as_str()
            .unwrap()
            .contains("explicit_language"));
        assert!(!body.to_string().contains("secret-key"));
    }

    #[test]
    fn parser_decodes_base64_audio_across_http_chunk_boundaries() {
        let mut parser = TtsStreamParser::default();
        assert!(parser.push(br#"{"code":0,"data":"AQ"#).unwrap().is_empty());
        assert_eq!(
            parser.push(br#"I="}"#).unwrap(),
            vec![TtsStreamEvent::Audio(vec![1, 2])]
        );
        assert_eq!(
            parser.push(b"\n{\"code\":20000000}\n").unwrap(),
            vec![TtsStreamEvent::Finished]
        );
        assert!(parser.finish().unwrap().is_empty());
    }

    #[test]
    fn parser_redacts_provider_message() {
        let mut parser = TtsStreamParser::default();
        let error = parser
            .push(br#"{"code":45000000,"message":"never show this"}"#)
            .unwrap_err();

        assert!(error.contains("音色"));
        assert!(!error.contains("never show this"));
    }

    #[test]
    fn long_text_is_split_without_losing_or_reordering_characters() {
        let text = "字".repeat(1_100);
        let segments = split_text(&text);

        assert!(segments.len() >= 3);
        assert!(segments
            .iter()
            .all(|segment| segment.chars().count() <= 450));
        assert_eq!(segments.concat(), text);
    }

    #[test]
    fn long_text_prefers_sentence_boundaries() {
        let text = format!("{}。后一句", "字".repeat(120));
        let segments = split_text(&text);

        assert_eq!(
            segments,
            vec![format!("{}。", "字".repeat(120)), "后一句".to_owned()]
        );
    }
}
