use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Serialize;

#[cfg(target_os = "macos")]
const DEFAULT_WECOM_BUNDLE_ID: &str = "com.tencent.WeWorkMac";
#[cfg(target_os = "macos")]
const DEFAULT_WECOM_PROCESS_NAME: &str = "企业微信";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalFriendStatus {
    Added,
    Pending,
    AlreadyExists,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SendFriendStatus {
    Sent,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FriendMessageType {
    Text,
    Image,
    File,
}

#[derive(Debug, Clone)]
pub struct AddExternalFriendRequest {
    pub phone: String,
    pub remark: Option<String>,
    pub greeting: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AddExternalFriendResult {
    pub phone: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remark: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub greeting: Option<String>,
    pub status: ExternalFriendStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone)]
pub enum FriendMessagePayload {
    Text(String),
    Image(PathBuf),
    File(PathBuf),
}

impl FriendMessagePayload {
    pub fn message_type(&self) -> FriendMessageType {
        match self {
            FriendMessagePayload::Text(_) => FriendMessageType::Text,
            FriendMessagePayload::Image(_) => FriendMessageType::Image,
            FriendMessagePayload::File(_) => FriendMessageType::File,
        }
    }

    pub fn file_path(&self) -> Option<&Path> {
        match self {
            FriendMessagePayload::Text(_) => None,
            FriendMessagePayload::Image(path) | FriendMessagePayload::File(path) => Some(path),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SendFriendMessageRequest {
    pub to: String,
    pub payload: FriendMessagePayload,
}

#[derive(Debug, Clone, Serialize)]
pub struct SendFriendMessageResult {
    pub to: String,
    pub message_type: FriendMessageType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    pub status: SendFriendStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopChatMessage {
    pub key: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopRecentChatTarget {
    pub name: String,
    pub signature: String,
    pub preview: Option<String>,
}

pub trait WeComDesktopDriver: Send + Sync {
    fn add_external_friend(
        &self,
        request: &AddExternalFriendRequest,
    ) -> Result<AddExternalFriendResult>;

    fn send_friend_message(
        &self,
        request: &SendFriendMessageRequest,
    ) -> Result<SendFriendMessageResult>;
}

pub fn default_driver() -> Box<dyn WeComDesktopDriver> {
    #[cfg(target_os = "macos")]
    {
        Box::new(MacosWeComDesktopDriver)
    }

    #[cfg(not(target_os = "macos"))]
    {
        Box::new(UnsupportedWeComDesktopDriver)
    }
}

pub fn validate_phone(phone: &str) -> Result<String> {
    let normalized = phone
        .trim()
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '(' | ')'))
        .collect::<String>();

    if normalized.is_empty() {
        bail!("手机号不能为空");
    }

    let plus_count = normalized.chars().filter(|c| *c == '+').count();
    let digits = normalized.strip_prefix('+').unwrap_or(&normalized);
    let has_valid_plus = plus_count == 0 || (plus_count == 1 && normalized.starts_with('+'));
    if !has_valid_plus || digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        bail!("手机号只能包含数字，或以 + 开头表示国际区号");
    }

    if !(5..=20).contains(&digits.len()) {
        bail!("手机号长度必须为 5 到 20 位数字");
    }

    Ok(normalized)
}

pub fn validate_target(target: &str) -> Result<String> {
    let target = target.trim();
    if target.is_empty() {
        bail!("发送对象不能为空");
    }
    if target.len() > 256 {
        bail!("发送对象过长，最大 256 字节");
    }
    Ok(target.to_string())
}

pub fn validate_text_message(text: &str) -> Result<String> {
    if text.trim().is_empty() {
        bail!("消息文本不能为空");
    }
    if text.len() > 2048 {
        bail!("消息文本过长，最大 2048 字节");
    }
    Ok(text.to_string())
}

pub fn validate_existing_file(path: &Path, usage: FileUsage) -> Result<PathBuf> {
    let canonical = path
        .canonicalize()
        .with_context(|| format!("文件不存在或不可访问: {}", path.display()))?;
    let metadata = std::fs::metadata(&canonical)
        .with_context(|| format!("读取文件元信息失败: {}", canonical.display()))?;
    if !metadata.is_file() {
        bail!("路径不是文件: {}", canonical.display());
    }

    if usage == FileUsage::Image {
        let mime = mime_guess::from_path(&canonical).first_or_octet_stream();
        if mime.type_() != mime::IMAGE {
            bail!(
                "图片路径的文件类型不是 image/*: {} ({})",
                canonical.display(),
                mime
            );
        }
    }

    Ok(canonical)
}

pub fn read_friend_text_messages(target: &str) -> Result<Vec<DesktopChatMessage>> {
    validate_target(target)?;
    read_friend_text_messages_impl(target)
}

pub fn read_recent_chat_target_summaries(limit: usize) -> Result<Vec<DesktopRecentChatTarget>> {
    read_recent_chat_target_summaries_impl(limit)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileUsage {
    Image,
    File,
}

fn split_script_status(raw: &str) -> (&str, Option<String>) {
    let trimmed = raw.trim();
    match trimmed.split_once('|') {
        Some((status, detail)) => {
            let detail = detail.trim();
            (
                status.trim(),
                if detail.is_empty() {
                    None
                } else {
                    Some(detail.to_string())
                },
            )
        }
        None => (trimmed, None),
    }
}

#[cfg(target_os = "macos")]
struct MacosWeComDesktopDriver;

#[cfg(target_os = "macos")]
impl MacosWeComDesktopDriver {
    fn add_friend_script(request: &AddExternalFriendRequest) -> String {
        let process = applescript_string_literal(&process_name());
        let bundle_id = applescript_string_literal(&bundle_id());
        let phone = applescript_string_literal(&request.phone);
        let remark = applescript_string_literal(request.remark.as_deref().unwrap_or(""));
        let greeting = applescript_string_literal(request.greeting.as_deref().unwrap_or(""));

        format!(
            r#"
{handlers}
set processName to {process}
set phoneNumber to {phone}
set remarkText to {remark}
set greetingText to {greeting}

tell application id {bundle_id} to activate
delay 1
tell application "System Events"
    if not (exists process processName) then error "企业微信进程未启动"
    tell process processName to set frontmost to true
    keystroke "f" using {{command down}}
end tell
delay 0.4
pasteText(phoneNumber)
delay 1

set duplicateCount to countExactStaticTexts(processName, phoneNumber)
if duplicateCount > 1 then return "failed|搜索结果存在多个同手机号候选，请在企业微信中确认唯一联系人后重试"

tell application "System Events" to key code 36
delay 1

set existingButton to clickFirstButton(processName, {{"发消息", "发送消息"}})
if existingButton is not "" then return "already_exists|联系人已存在，可直接发消息"

set addButton to clickFirstButton(processName, {{"添加", "添加联系人", "添加到通讯录", "添加为联系人", "添加客户", "添加外部联系人"}})
if addButton is "" then return "failed|未找到添加外部联系人入口；可能无搜索结果、无权限、联系人已存在或客户端 UI 已变化"
delay 0.8

if greetingText is not "" then
    pasteText(greetingText)
    delay 0.2
end if

if remarkText is not "" then
    tell application "System Events" to key code 48
    delay 0.2
    pasteText(remarkText)
    delay 0.2
end if

set submitButton to clickFirstButton(processName, {{"发送", "确定", "完成", "申请添加"}})
if submitButton is "" then return "pending|已打开添加外部联系人流程，请在企业微信窗口手动确认"

return "pending|已提交或打开添加验证流程；对方确认前不会视为已添加"
"#,
            handlers = common_applescript_handlers(),
            process = process,
            phone = phone,
            remark = remark,
            greeting = greeting,
            bundle_id = bundle_id,
        )
    }

    fn send_message_script(request: &SendFriendMessageRequest) -> String {
        let process = applescript_string_literal(&process_name());
        let bundle_id = applescript_string_literal(&bundle_id());
        let target = applescript_string_literal(&request.to);
        let send_payload = send_payload_script(&request.payload);

        format!(
            r#"
{handlers}
set processName to {process}
set targetName to {target}

tell application id {bundle_id} to activate
delay 1
tell application "System Events"
    if not (exists process processName) then error "企业微信进程未启动"
    tell process processName to set frontmost to true
    keystroke "f" using {{command down}}
end tell
delay 0.4
pasteText(targetName)
delay 1

set duplicateCount to countExactStaticTexts(processName, targetName)
if duplicateCount > 1 then return "failed|搜索结果存在多个同名候选，请使用唯一备注名后重试"

tell application "System Events" to key code 36
delay 0.8
{send_payload}

return "sent|已触发企业微信桌面发送，并完成发送前校验"
"#,
            handlers = common_applescript_handlers(),
            process = process,
            target = target,
            bundle_id = bundle_id,
            send_payload = send_payload,
        )
    }
}

#[cfg(target_os = "macos")]
fn send_payload_script(payload: &FriendMessagePayload) -> String {
    match payload {
        FriendMessagePayload::Text(text) => format!(
            r#"pasteText({})
delay 0.4
tell application "System Events" to key code 36"#,
            applescript_string_literal(text)
        ),
        FriendMessagePayload::Image(path) => format!(
            r#"pasteFile({})
delay 1.2
sendActiveMessage(processName)"#,
            applescript_string_literal(&path.to_string_lossy())
        ),
        FriendMessagePayload::File(path) => format!(
            r#"pasteFile({})
delay 1.2
sendActiveMessage(processName)"#,
            applescript_string_literal(&path.to_string_lossy())
        ),
    }
}

#[cfg(target_os = "macos")]
impl WeComDesktopDriver for MacosWeComDesktopDriver {
    fn add_external_friend(
        &self,
        request: &AddExternalFriendRequest,
    ) -> Result<AddExternalFriendResult> {
        let raw = run_osascript(&Self::add_friend_script(request))?;
        let (status, detail) = split_script_status(&raw);
        let status = match status {
            "added" => ExternalFriendStatus::Added,
            "pending" => ExternalFriendStatus::Pending,
            "already_exists" => ExternalFriendStatus::AlreadyExists,
            "failed" => ExternalFriendStatus::Failed,
            other => {
                return Ok(AddExternalFriendResult {
                    phone: request.phone.clone(),
                    remark: request.remark.clone(),
                    greeting: request.greeting.clone(),
                    status: ExternalFriendStatus::Failed,
                    detail: Some(format!("无法识别桌面自动化结果: {other}")),
                });
            }
        };

        Ok(AddExternalFriendResult {
            phone: request.phone.clone(),
            remark: request.remark.clone(),
            greeting: request.greeting.clone(),
            status,
            detail,
        })
    }

    fn send_friend_message(
        &self,
        request: &SendFriendMessageRequest,
    ) -> Result<SendFriendMessageResult> {
        let raw = run_osascript(&Self::send_message_script(request))?;
        let (status, detail) = split_script_status(&raw);
        let status = match status {
            "sent" => SendFriendStatus::Sent,
            "failed" => SendFriendStatus::Failed,
            other => {
                return Ok(SendFriendMessageResult {
                    to: request.to.clone(),
                    message_type: request.payload.message_type(),
                    file_path: request
                        .payload
                        .file_path()
                        .map(|path| path.to_string_lossy().to_string()),
                    status: SendFriendStatus::Failed,
                    detail: Some(format!("无法识别桌面自动化结果: {other}")),
                });
            }
        };

        Ok(SendFriendMessageResult {
            to: request.to.clone(),
            message_type: request.payload.message_type(),
            file_path: request
                .payload
                .file_path()
                .map(|path| path.to_string_lossy().to_string()),
            status,
            detail,
        })
    }
}

#[cfg(not(target_os = "macos"))]
struct UnsupportedWeComDesktopDriver;

#[cfg(not(target_os = "macos"))]
impl WeComDesktopDriver for UnsupportedWeComDesktopDriver {
    fn add_external_friend(
        &self,
        _request: &AddExternalFriendRequest,
    ) -> Result<AddExternalFriendResult> {
        bail!("当前平台不支持 macOS 企业微信桌面自动化，请在 macOS 上运行该命令")
    }

    fn send_friend_message(
        &self,
        _request: &SendFriendMessageRequest,
    ) -> Result<SendFriendMessageResult> {
        bail!("当前平台不支持 macOS 企业微信桌面自动化，请在 macOS 上运行该命令")
    }
}

#[cfg(target_os = "macos")]
fn process_name() -> String {
    std::env::var("WECOM_CLI_DESKTOP_PROCESS")
        .unwrap_or_else(|_| DEFAULT_WECOM_PROCESS_NAME.to_string())
}

#[cfg(target_os = "macos")]
fn bundle_id() -> String {
    std::env::var("WECOM_CLI_DESKTOP_BUNDLE_ID")
        .unwrap_or_else(|_| DEFAULT_WECOM_BUNDLE_ID.to_string())
}

#[cfg(target_os = "macos")]
fn run_osascript(script: &str) -> Result<String> {
    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .context("执行 macOS 桌面自动化脚本失败")?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.status.success() {
        let detail = if stderr.is_empty() {
            "请确认企业微信已登录，且终端已授予“辅助功能”和“自动化”权限".to_string()
        } else {
            stderr
        };
        bail!("企业微信桌面自动化失败: {detail}");
    }

    Ok(stdout)
}

#[cfg(target_os = "macos")]
fn read_friend_text_messages_impl(target: &str) -> Result<Vec<DesktopChatMessage>> {
    let process = applescript_string_literal(&process_name());
    let target = applescript_string_literal(target);
    let script = format!(
        r#"
{handlers}
set processName to {process}
set targetName to {target}

tell application {process} to activate
delay 0.15
tell application "System Events"
    if not (exists process processName) then error "企业微信进程未启动"
    tell process processName to set frontmost to true
end tell

if my clickRecentConversation(processName, targetName) then
    delay 0.15
else
    tell application "System Events"
        tell process processName to set frontmost to true
        keystroke "f" using {{command down}}
    end tell
    delay 0.2
    pasteText(targetName)
    delay 0.4
    tell application "System Events" to key code 36
    delay 0.8
end if

set outputLines to {{}}
tell application "System Events"
    tell process processName
        set mainWindow to my mainWeComWindow(processName)
        set textIndex to 0
        try
            set uiElements to entire contents of scroll area 1 of splitter group 1 of splitter group 1 of splitter group 1 of splitter group 1 of mainWindow
        on error
            set uiElements to entire contents of mainWindow
        end try
        repeat with itemElement in uiElements
            try
                if (role of itemElement as text) is "AXTextArea" then
                    set rawText to value of itemElement as text
                    set cleanText to my trimText(rawText)
                    if cleanText is not "" then
                        set itemPosition to position of itemElement as text
                        set itemSize to size of itemElement as text
                        set textIndex to textIndex + 1
                        set end of outputLines to (textIndex as text) & tab & my sanitizeLine(itemPosition) & tab & my sanitizeLine(itemSize) & tab & my sanitizeLine(cleanText)
                    end if
                end if
            end try
        end repeat
    end tell
end tell

set AppleScript's text item delimiters to linefeed
set joinedOutput to outputLines as text
set AppleScript's text item delimiters to ""
return joinedOutput
"#,
        handlers = desktop_read_applescript_handlers(),
        process = process,
        target = target,
    );

    parse_desktop_chat_messages(&run_osascript(&script)?)
}

#[cfg(not(target_os = "macos"))]
fn read_friend_text_messages_impl(_target: &str) -> Result<Vec<DesktopChatMessage>> {
    bail!("当前平台不支持 macOS 企业微信桌面消息读取，请在 macOS 上运行该命令")
}

#[cfg(target_os = "macos")]
fn read_recent_chat_target_summaries_impl(limit: usize) -> Result<Vec<DesktopRecentChatTarget>> {
    let process = applescript_string_literal(&process_name());
    let limit = limit.max(1);
    let script = format!(
        r#"
{handlers}
set processName to {process}
set maxTargets to {limit}

tell application {process} to activate
delay 0.15
tell application "System Events"
    if not (exists process processName) then error "企业微信进程未启动"
    tell process processName
        set frontmost to true
        key code 53
        delay 0.05
        key code 53
    end tell
end tell
delay 0.1

set outputLines to {{}}
tell application "System Events"
    tell process processName
        set mainWindow to my mainWeComWindow(processName)
        set textIndex to 0
        try
            set uiElements to entire contents of scroll area 1 of splitter group 1 of splitter group 1 of mainWindow
        on error
            set uiElements to entire contents of mainWindow
        end try
        repeat with itemElement in uiElements
            try
                if (role of itemElement as text) is "AXStaticText" then
                    set rawText to value of itemElement as text
                    set cleanText to my trimText(rawText)
                    if cleanText is not "" then
                        set textIndex to textIndex + 1
                        set end of outputLines to (textIndex as text) & tab & my sanitizeLine(cleanText)
                    end if
                end if
            end try
        end repeat
    end tell
end tell

set AppleScript's text item delimiters to linefeed
set joinedOutput to outputLines as text
set AppleScript's text item delimiters to ""
return joinedOutput
"#,
        handlers = desktop_read_applescript_handlers(),
        process = process,
        limit = limit,
    );

    parse_recent_chat_target_summaries(&run_osascript(&script)?, limit)
}

#[cfg(not(target_os = "macos"))]
fn read_recent_chat_target_summaries_impl(_limit: usize) -> Result<Vec<DesktopRecentChatTarget>> {
    bail!("当前平台不支持 macOS 企业微信桌面会话列表读取，请在 macOS 上运行该命令")
}

fn parse_desktop_chat_messages(raw: &str) -> Result<Vec<DesktopChatMessage>> {
    let mut messages = Vec::new();
    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        let parts = line.splitn(4, '\t').collect::<Vec<_>>();
        if parts.len() != 4 {
            bail!("解析桌面消息失败: {line}");
        }
        let text = parts[3].replace("\\n", "\n");
        messages.push(DesktopChatMessage {
            key: format!("{}|{}", parts[0], text),
            text,
        });
    }

    Ok(messages)
}

#[cfg(test)]
fn parse_recent_chat_targets(raw: &str, limit: usize) -> Result<Vec<String>> {
    Ok(parse_recent_chat_target_summaries(raw, limit)?
        .into_iter()
        .map(|target| target.name)
        .collect())
}

fn parse_recent_chat_target_summaries(
    raw: &str,
    limit: usize,
) -> Result<Vec<DesktopRecentChatTarget>> {
    if raw
        .lines()
        .filter(|line| !line.trim().is_empty())
        .all(|line| line.split('\t').count() == 2)
    {
        return parse_recent_chat_target_summaries_from_text_sequence(raw, limit);
    }

    #[derive(Debug)]
    struct TextItem {
        x: i64,
        y: i64,
        text: String,
    }

    let mut items = Vec::new();
    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        let parts = line.splitn(3, '\t').collect::<Vec<_>>();
        if parts.len() != 3 {
            continue;
        }
        let Ok(x) = parts[0].parse::<i64>() else {
            continue;
        };
        let Ok(y) = parts[1].parse::<i64>() else {
            continue;
        };
        items.push(TextItem {
            x,
            y,
            text: parts[2].trim().to_string(),
        });
    }

    let mut marker_rows = items
        .iter()
        .filter(|item| item.text == "@微信")
        .map(|item| item.y)
        .collect::<Vec<_>>();
    marker_rows.sort_unstable();
    marker_rows.dedup();

    let mut targets = Vec::new();
    for marker_y in marker_rows.iter().copied() {
        if targets.len() >= limit {
            break;
        }

        let Some(name_item) = items
            .iter()
            .filter(|item| {
                !item.text.is_empty()
                    && item.text != "@微信"
                    && item.x <= 260
                    && (item.y - marker_y).abs() <= 8
            })
            .min_by_key(|item| (item.x, item.y))
        else {
            continue;
        };

        if targets
            .iter()
            .any(|target: &DesktopRecentChatTarget| target.name == name_item.text)
        {
            continue;
        }

        let row_end = marker_rows
            .iter()
            .copied()
            .find(|next_y| *next_y > marker_y)
            .map(|next_y| next_y - 8)
            .unwrap_or(marker_y + 52);
        let preview = items
            .iter()
            .filter(|item| {
                item.y > marker_y + 8
                    && item.y < row_end
                    && item.x <= 260
                    && item.text != "@微信"
                    && item.text != name_item.text
            })
            .min_by_key(|item| (item.y, item.x))
            .map(|item| item.text.clone());
        targets.push(make_recent_chat_target(name_item.text.clone(), preview));
    }

    Ok(targets)
}

fn parse_recent_chat_target_summaries_from_text_sequence(
    raw: &str,
    limit: usize,
) -> Result<Vec<DesktopRecentChatTarget>> {
    let mut groups = Vec::new();
    let mut current = Vec::new();

    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        let parts = line.splitn(2, '\t').collect::<Vec<_>>();
        if parts.len() != 2 {
            continue;
        }
        let text = parts[1].trim();
        if text.is_empty() {
            continue;
        }

        current.push(text.to_string());
        if text == "@微信" {
            groups.push(std::mem::take(&mut current));
        }
    }

    let mut targets = Vec::new();
    for group in groups {
        if targets.len() >= limit {
            break;
        }

        let row_texts = group
            .into_iter()
            .filter(|text| text != "@微信")
            .collect::<Vec<_>>();
        let Some(name) = row_texts.first().filter(|name| !name.is_empty()) else {
            continue;
        };

        if targets
            .iter()
            .any(|target: &DesktopRecentChatTarget| target.name == *name)
        {
            continue;
        }

        let preview = recent_chat_preview_from_row(&row_texts);
        targets.push(make_recent_chat_target(name.clone(), preview));
    }

    Ok(targets)
}

fn recent_chat_preview_from_row(row_texts: &[String]) -> Option<String> {
    if row_texts.len() >= 3 {
        return row_texts.get(1).filter(|text| !text.is_empty()).cloned();
    }

    row_texts
        .get(1)
        .filter(|text| !looks_like_recent_chat_time(text))
        .cloned()
}

fn make_recent_chat_target(name: String, preview: Option<String>) -> DesktopRecentChatTarget {
    let signature = format!("{}|{}", name, preview.as_deref().unwrap_or(""));
    DesktopRecentChatTarget {
        name,
        signature,
        preview,
    }
}

fn looks_like_recent_chat_time(text: &str) -> bool {
    text.contains("分钟前")
        || text.contains("小时前")
        || text == "昨天"
        || text == "星期一"
        || text == "星期二"
        || text == "星期三"
        || text == "星期四"
        || text == "星期五"
        || text == "星期六"
        || text == "星期日"
        || text
            .chars()
            .all(|c| c.is_ascii_digit() || c == ':' || c == '/')
}

#[cfg(target_os = "macos")]
fn applescript_string_literal(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

#[cfg(target_os = "macos")]
fn common_applescript_handlers() -> &'static str {
    r#"
use framework "AppKit"
use scripting additions

on pasteText(valueText)
    set the clipboard to valueText
    delay 0.2
    tell application "System Events"
        keystroke "v" using {command down}
    end tell
end pasteText

on pasteFile(posixPath)
    set fileUrl to current application's NSURL's fileURLWithPath:posixPath
    set pasteboard to current application's NSPasteboard's generalPasteboard()
    pasteboard's clearContents()
    set didWrite to pasteboard's writeObjects:{fileUrl}
    if (didWrite as boolean) is false then error "写入文件剪贴板失败"
    delay 0.2
    tell application "System Events"
        keystroke "v" using {command down}
    end tell
end pasteFile

on countExactStaticTexts(processName, expectedText)
    tell application "System Events"
        tell process processName
            set hitCount to 0
            try
                set mainWindow to my mainWeComWindow(processName)
                set uiElements to entire contents of mainWindow
                set uiTexts to {}
                repeat with itemElement in uiElements
                    try
                        if (role of itemElement as text) is "AXStaticText" then set end of uiTexts to itemElement
                    end try
                end repeat
                repeat with itemText in uiTexts
                    try
                        set actualText to value of itemText as text
                        if actualText is expectedText then set hitCount to hitCount + 1
                    end try
                end repeat
            end try
            return hitCount
        end tell
    end tell
end countExactStaticTexts

on clickFirstButton(processName, candidates)
    tell application "System Events"
        tell process processName
            set mainWindow to my mainWeComWindow(processName)
            repeat with buttonName in candidates
                try
                    click (first button of mainWindow whose name is (buttonName as text))
                    return buttonName as text
                end try
            end repeat
        end tell
    end tell
    return ""
end clickFirstButton

on sendActiveMessage(processName)
    set clickedButton to clickFirstButton(processName, {"发送"})
    if clickedButton is "" then
        tell application "System Events" to key code 36
    end if
end sendActiveMessage

on replaceText(sourceText, searchText, replacementText)
    set oldDelimiters to AppleScript's text item delimiters
    set AppleScript's text item delimiters to searchText
    set textParts to text items of sourceText
    set AppleScript's text item delimiters to replacementText
    set replacedText to textParts as text
    set AppleScript's text item delimiters to oldDelimiters
    return replacedText
end replaceText

on trimText(sourceText)
    set trimmedText to sourceText as text
    repeat while trimmedText begins with space or trimmedText begins with tab or trimmedText begins with linefeed or trimmedText begins with return
        if length of trimmedText is 1 then return ""
        set trimmedText to text 2 thru -1 of trimmedText
    end repeat
    repeat while trimmedText ends with space or trimmedText ends with tab or trimmedText ends with linefeed or trimmedText ends with return
        if length of trimmedText is 1 then return ""
        set trimmedText to text 1 thru -2 of trimmedText
    end repeat
    return trimmedText
end trimText

on sanitizeLine(sourceText)
    set sanitizedText to sourceText as text
    set sanitizedText to replaceText(sanitizedText, tab, " ")
    set sanitizedText to replaceText(sanitizedText, linefeed, "\\n")
    set sanitizedText to replaceText(sanitizedText, return, "\\n")
    return sanitizedText
end sanitizeLine

on absNumber(sourceNumber)
    if sourceNumber < 0 then return -sourceNumber
    return sourceNumber
end absNumber

on clickRecentConversation(processName, targetName)
    tell application "System Events"
        tell process processName
            try
                set mainWindow to my mainWeComWindow(processName)
                set listElements to entire contents of scroll area 1 of splitter group 1 of splitter group 1 of mainWindow
                repeat with itemElement in listElements
                    try
                        if (role of itemElement as text) is "AXStaticText" then
                            set rawText to value of itemElement as text
                            if rawText is targetName then
                                click itemElement
                                return true
                            end if
                        end if
                    end try
                end repeat
            end try
        end tell
    end tell
    return false
end clickRecentConversation

on isTargetConversationOpen(processName, targetName)
    tell application "System Events"
        tell process processName
            try
                set mainWindow to my mainWeComWindow(processName)
                set uiElements to entire contents of mainWindow
                repeat with itemElement in uiElements
                    try
                        if (role of itemElement as text) is "AXStaticText" then
                            set rawText to value of itemElement as text
                            if rawText is targetName then
                                set itemPosition to position of itemElement
                                set xPos to item 1 of itemPosition
                                set yPos to item 2 of itemPosition
                                if xPos > 300 and yPos > 40 and yPos < 140 then return true
                            end if
                        end if
                    end try
                end repeat
            end try
        end tell
    end tell
    return false
end isTargetConversationOpen

on mainWeComWindow(processName)
    tell application "System Events"
        tell process processName
            set bestWindow to missing value
            set bestArea to 0
            repeat with itemWindow in windows
                try
                    set itemSize to size of itemWindow
                    set itemWidth to item 1 of itemSize
                    set itemHeight to item 2 of itemSize
                    set itemArea to itemWidth * itemHeight
                    if itemArea > bestArea then
                        set bestArea to itemArea
                        set bestWindow to itemWindow
                    end if
                end try
            end repeat
            if bestWindow is missing value then error "未找到企业微信主窗口"
            return bestWindow
        end tell
    end tell
end mainWeComWindow
"#
}

#[cfg(target_os = "macos")]
fn desktop_read_applescript_handlers() -> &'static str {
    r#"
on pasteText(valueText)
    set the clipboard to valueText
    delay 0.2
    tell application "System Events"
        keystroke "v" using {command down}
    end tell
end pasteText

on replaceText(sourceText, searchText, replacementText)
    set oldDelimiters to AppleScript's text item delimiters
    set AppleScript's text item delimiters to searchText
    set textParts to text items of sourceText
    set AppleScript's text item delimiters to replacementText
    set replacedText to textParts as text
    set AppleScript's text item delimiters to oldDelimiters
    return replacedText
end replaceText

on trimText(sourceText)
    set trimmedText to sourceText as text
    repeat while trimmedText begins with space or trimmedText begins with tab or trimmedText begins with linefeed or trimmedText begins with return
        if length of trimmedText is 1 then return ""
        set trimmedText to text 2 thru -1 of trimmedText
    end repeat
    repeat while trimmedText ends with space or trimmedText ends with tab or trimmedText ends with linefeed or trimmedText ends with return
        if length of trimmedText is 1 then return ""
        set trimmedText to text 1 thru -2 of trimmedText
    end repeat
    return trimmedText
end trimText

on sanitizeLine(sourceText)
    set sanitizedText to sourceText as text
    set sanitizedText to replaceText(sanitizedText, tab, " ")
    set sanitizedText to replaceText(sanitizedText, linefeed, "\\n")
    set sanitizedText to replaceText(sanitizedText, return, "\\n")
    return sanitizedText
end sanitizeLine

on absNumber(sourceNumber)
    if sourceNumber < 0 then return -sourceNumber
    return sourceNumber
end absNumber

on clickRecentConversation(processName, targetName)
    tell application "System Events"
        tell process processName
            try
                set mainWindow to my mainWeComWindow(processName)
                set listElements to entire contents of scroll area 1 of splitter group 1 of splitter group 1 of mainWindow
                repeat with itemElement in listElements
                    try
                        if (role of itemElement as text) is "AXStaticText" then
                            set rawText to value of itemElement as text
                            if rawText is targetName then
                                click itemElement
                                return true
                            end if
                        end if
                    end try
                end repeat
            end try
        end tell
    end tell
    return false
end clickRecentConversation

on isTargetConversationOpen(processName, targetName)
    tell application "System Events"
        tell process processName
            try
                set mainWindow to my mainWeComWindow(processName)
                set uiElements to entire contents of mainWindow
                repeat with itemElement in uiElements
                    try
                        if (role of itemElement as text) is "AXStaticText" then
                            set rawText to value of itemElement as text
                            if rawText is targetName then
                                set itemPosition to position of itemElement
                                set xPos to item 1 of itemPosition
                                set yPos to item 2 of itemPosition
                                if xPos > 300 and yPos > 40 and yPos < 140 then return true
                            end if
                        end if
                    end try
                end repeat
            end try
        end tell
    end tell
    return false
end isTargetConversationOpen

on mainWeComWindow(processName)
    tell application "System Events"
        tell process processName
            set bestWindow to missing value
            set bestArea to 0
            repeat with itemWindow in windows
                try
                    set itemSize to size of itemWindow
                    set itemWidth to item 1 of itemSize
                    set itemHeight to item 2 of itemSize
                    set itemArea to itemWidth * itemHeight
                    if itemArea > bestArea then
                        set bestArea to itemArea
                        set bestWindow to itemWindow
                    end if
                end try
            end repeat
            if bestWindow is missing value then error "未找到企业微信主窗口"
            return bestWindow
        end tell
    end tell
end mainWeComWindow
"#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_phone_accepts_digits_and_country_code() {
        assert_eq!(validate_phone("13800000000").unwrap(), "13800000000");
        assert_eq!(
            validate_phone("+86 138-0000-0000").unwrap(),
            "+8613800000000"
        );
    }

    #[test]
    fn validate_phone_rejects_invalid_input() {
        assert!(validate_phone("").is_err());
        assert!(validate_phone("abc").is_err());
        assert!(validate_phone("++8613800000000").is_err());
        assert!(validate_phone("1234").is_err());
    }

    #[test]
    fn validate_text_message_checks_empty_and_length() {
        assert!(validate_text_message("hello").is_ok());
        assert!(validate_text_message("   ").is_err());
        assert!(validate_text_message(&"a".repeat(2049)).is_err());
    }

    #[test]
    fn parse_desktop_chat_messages_reads_tab_delimited_rows() {
        let parsed = parse_desktop_chat_messages("1\t341, 42\t20, 22\t你好\\n呀\n").unwrap();
        assert_eq!(
            parsed,
            vec![DesktopChatMessage {
                key: "1|你好\n呀".to_string(),
                text: "你好\n呀".to_string(),
            }]
        );
    }

    #[test]
    fn parse_recent_chat_targets_skips_empty_and_duplicate_lines() {
        let parsed = parse_recent_chat_targets(
            "159\t143\t邹友\n191\t144\t@微信\n159\t164\t消息预览\n159\t207\t友友\n191\t208\t@微信\n159\t271\t行业资讯\n",
            12,
        )
        .unwrap();

        assert_eq!(parsed, vec!["邹友".to_string(), "友友".to_string()]);
    }

    #[test]
    fn parse_recent_chat_target_summaries_includes_preview_signature() {
        let parsed = parse_recent_chat_target_summaries(
            "159\t143\t邹友\n191\t144\t@微信\n159\t164\t新消息\n159\t207\t友友\n191\t208\t@微信\n159\t228\t旧消息\n",
            12,
        )
        .unwrap();

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "邹友");
        assert_eq!(parsed[0].preview.as_deref(), Some("新消息"));
        assert!(parsed[0].signature.contains("新消息"));
        assert!(!parsed[0].signature.contains("rank:"));
        assert_eq!(parsed[1].name, "友友");
        assert_eq!(parsed[1].preview.as_deref(), Some("旧消息"));
        assert!(parsed[1].signature.contains("旧消息"));
    }

    #[test]
    fn parse_recent_chat_target_summaries_reads_fast_text_sequence() {
        let parsed = parse_recent_chat_target_summaries(
            "1\t邹友\n2\t可以快点吗\n3\t12分钟前\n4\t@微信\n5\t友友\n6\t哈哈哈\n7\t13分钟前\n8\t@微信\n",
            12,
        )
        .unwrap();

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "邹友");
        assert_eq!(parsed[0].preview.as_deref(), Some("可以快点吗"));
        assert_eq!(parsed[1].name, "友友");
        assert_eq!(parsed[1].preview.as_deref(), Some("哈哈哈"));
    }

    #[test]
    fn parse_recent_chat_target_summaries_keeps_time_like_preview() {
        let parsed = parse_recent_chat_target_summaries(
            "1\t邹友\n2\t14:08:03\n3\t刚刚\n4\t@微信\n5\t友友\n6\t5/7\n7\t@微信\n",
            12,
        )
        .unwrap();

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "邹友");
        assert_eq!(parsed[0].preview.as_deref(), Some("14:08:03"));
        assert_eq!(parsed[1].name, "友友");
        assert_eq!(parsed[1].preview, None);
    }

    #[test]
    fn parse_recent_chat_target_signature_ignores_row_order() {
        let first = parse_recent_chat_target_summaries(
            "159\t143\t邹友\n191\t144\t@微信\n159\t164\t新消息\n159\t207\t友友\n191\t208\t@微信\n159\t228\t旧消息\n",
            12,
        )
        .unwrap();
        let second = parse_recent_chat_target_summaries(
            "159\t143\t友友\n191\t144\t@微信\n159\t164\t旧消息\n159\t207\t邹友\n191\t208\t@微信\n159\t228\t新消息\n",
            12,
        )
        .unwrap();

        let first_zou = first.iter().find(|target| target.name == "邹友").unwrap();
        let second_zou = second.iter().find(|target| target.name == "邹友").unwrap();

        assert_eq!(first_zou.signature, second_zou.signature);
    }

    #[test]
    fn split_script_status_parses_detail() {
        assert_eq!(
            split_script_status("pending|等待确认"),
            ("pending", Some("等待确认".to_string()))
        );
        assert_eq!(split_script_status("sent"), ("sent", None));
    }

    #[test]
    fn validate_existing_image_rejects_non_image_extension() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.pdf");
        std::fs::write(&path, b"not an image").unwrap();

        assert!(validate_existing_file(&path, FileUsage::Image).is_err());
        assert!(validate_existing_file(&path, FileUsage::File).is_ok());
    }
}
