mod utils;

use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

const INBOUND_MAX_BYTES: usize = 20 * 1024 * 1024;

/// Intercept a `get_msg_media` response: decode base64 payload, save to disk, and replace the response with a local file reference.
pub async fn intercept_media_response(res: Value) -> Result<Value> {
    Ok(intercept_media_response_inner(res, None).await?.response)
}

/// Save a `get_msg_media` response and return the concise `media_item` object when present.
pub async fn extract_media_item_to_dir(res: Value, save_dir: &Path) -> Result<Option<Value>> {
    Ok(intercept_media_response_inner(res, Some(save_dir))
        .await?
        .media_item)
}

struct MediaIntercept {
    response: Value,
    media_item: Option<Value>,
}

async fn intercept_media_response_inner(
    res: Value,
    save_dir: Option<&Path>,
) -> Result<MediaIntercept> {
    let Some(text) = extract_mcp_text(&res).map(ToString::to_string) else {
        return Ok(MediaIntercept {
            response: res,
            media_item: None,
        });
    };

    // Parse the business JSON
    let biz_data: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => {
            return Ok(MediaIntercept {
                response: res,
                media_item: None,
            });
        } // Not JSON format, return as-is
    };

    // 3. Validate business response: return as-is when errcode !== 0 or no media_item
    if biz_data.get("errcode").and_then(|c| c.as_i64()) != Some(0) {
        return Ok(MediaIntercept {
            response: res,
            media_item: None,
        });
    }

    let Some(media_item) = biz_data.get("media_item") else {
        return Ok(MediaIntercept {
            response: res,
            media_item: None,
        });
    };

    let Some(base64_data) = media_item.get("base64_data").and_then(|d| d.as_str()) else {
        return Ok(MediaIntercept {
            response: res,
            media_item: None,
        });
    };

    let media_name = media_item.get("name").and_then(|n| n.as_str());
    let media_type = media_item.get("type").and_then(|t| t.as_str());
    let media_id = media_item.get("media_id").and_then(|i| i.as_str());

    // 4. Decode base64 → buffer
    use base64::Engine as _;
    let buffer = base64::engine::general_purpose::STANDARD
        .decode(base64_data)
        .context("base64解码失败")?;

    // Validate size
    if buffer.len() > INBOUND_MAX_BYTES {
        bail!(
            "媒体文件过大: {} 字节 (最大 {} 字节)",
            buffer.len(),
            INBOUND_MAX_BYTES
        );
    }

    // 5. Detect MIME type
    let content_type = utils::detect_mime(media_name, &buffer);

    // 6. Save to local file
    let file_path = match save_dir {
        Some(dir) => {
            utils::save_media_to_dir(dir, media_name, media_id, &content_type, &buffer).await?
        }
        None => utils::save_media(media_name, media_id, &content_type, &buffer).await?,
    };

    // 7. Build a concise response: remove base64_data, add local path
    let new_media_item = json!({
        "media_id": media_id,
        "name": media_name.unwrap_or_else(|| file_path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")),
        "type": media_type,
        "local_path": file_path.to_string_lossy(),
        "size": buffer.len(),
        "content_type": content_type,
    });
    let new_biz_data = json!({
        "errcode": 0,
        "errmsg": "ok",
        "media_item": new_media_item,
    });

    // 8. Replace res in-place with the modified MCP result structure
    Ok(MediaIntercept {
        response: json!({
            "result": {
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string(&new_biz_data)?,
                }],
            },
        }),
        media_item: Some(new_biz_data["media_item"].clone()),
    })
}

fn extract_mcp_text(res: &Value) -> Option<&str> {
    res.get("result")?
        .get("content")?
        .as_array()?
        .iter()
        .find(|item| {
            item.get("type").and_then(Value::as_str) == Some("text")
                && item.get("text").and_then(Value::as_str).is_some()
        })?
        .get("text")?
        .as_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn extract_media_item_saves_to_custom_dir() {
        let dir = tempfile::tempdir().unwrap();
        let business = json!({
            "errcode": 0,
            "errmsg": "ok",
            "media_item": {
                "media_id": "MEDIAID_1",
                "name": "hello.txt",
                "type": "file",
                "base64_data": "aGVsbG8="
            }
        });
        let response = json!({
            "result": {
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string(&business).unwrap()
                }]
            }
        });

        let item = extract_media_item_to_dir(response, dir.path())
            .await
            .unwrap()
            .unwrap();
        let local_path = item
            .get("local_path")
            .and_then(Value::as_str)
            .map(Path::new)
            .unwrap();

        assert!(local_path.starts_with(dir.path()));
        assert_eq!(tokio::fs::read(local_path).await.unwrap(), b"hello");
    }
}
