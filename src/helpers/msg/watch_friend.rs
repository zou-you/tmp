use std::collections::BTreeSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::{Duration as StdDuration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use clap::{ArgMatches, Args, Command, FromArgMatches};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::fs_util::{atomic_write, sanitize_filename};
use crate::helpers::registry::Helper;
use crate::helpers::wecom_desktop::{
    DesktopChatMessage, read_friend_text_messages, validate_target,
};
use crate::{json_rpc, media, paths};

const MAX_PAGES: usize = 100;

#[derive(Args, Debug)]
pub struct WatchFriendArgs {
    /// 好友名称、备注名或 chat_id
    #[arg(long, required = true, num_args = 1, action = clap::ArgAction::Append)]
    pub to: Vec<String>,

    /// 轮询间隔秒数
    #[arg(long, default_value_t = 5)]
    pub interval_sec: u64,

    /// 图片和文件保存目录，默认使用 wecom-cli 媒体临时目录
    #[arg(long)]
    pub save_dir: Option<PathBuf>,

    /// 只轮询一次后退出，便于脚本和测试使用
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub once: bool,

    /// 输出达到指定条数后退出；0 表示不限制
    #[arg(long, default_value_t = 0)]
    pub max_events: usize,

    /// 没有新消息达到指定秒数后退出；0 表示不限制
    #[arg(long, default_value_t = 0)]
    pub idle_timeout_sec: u64,

    /// 打印轮询诊断信息到 stderr；stdout 仍只输出消息 NDJSON
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub verbose: bool,

    /// 自定义状态文件路径，默认写入 save-dir 下的隐藏 JSON 文件
    #[arg(long, hide = true)]
    pub state_file: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
struct ChatSummary {
    #[serde(default)]
    chat_id: String,
    #[serde(default)]
    chat_name: String,
    #[serde(default)]
    last_msg_time: Option<String>,
    #[serde(default)]
    msg_count: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
struct ContactUser {
    #[serde(default)]
    userid: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    alias: String,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct WatchState {
    seen: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WatchTargetMeta {
    target: String,
    chat_id: String,
    chat_name: String,
}

#[derive(Debug)]
struct WatchTarget {
    meta: WatchTargetMeta,
    state_file: PathBuf,
    state: WatchState,
}

#[derive(Debug, Clone)]
struct WatchWorkerOptions {
    save_dir: PathBuf,
    interval_sec: u64,
    once: bool,
    verbose: bool,
    desktop_fallback: bool,
}

#[derive(Debug)]
struct WatchOutput {
    output: Value,
    ack: oneshot::Sender<bool>,
}

#[derive(Debug)]
enum WatchEvent {
    Message(WatchOutput),
    Log(String),
}

pub struct WatchFriendHelper;

impl Helper for WatchFriendHelper {
    fn category(&self) -> &'static str {
        "msg"
    }

    fn command(&self) -> clap::Command {
        WatchFriendArgs::augment_args(
            Command::new("+watch_friend").about("轮询指定好友新消息，并保存图片和文件"),
        )
    }

    fn execute<'a>(
        &'a self,
        matches: &'a ArgMatches,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async {
            let args = WatchFriendArgs::from_arg_matches(matches)?;
            if args.interval_sec == 0 {
                bail!("--interval-sec 必须大于 0");
            }

            let save_dir = args.save_dir.unwrap_or_else(paths::media_dir);
            tokio::fs::create_dir_all(&save_dir)
                .await
                .with_context(|| format!("创建保存目录失败: {}", save_dir.display()))?;

            let targets =
                prepare_watch_targets(args.to, &save_dir, args.state_file, args.verbose).await?;
            let desktop_fallback = targets.len() == 1;
            if args.verbose {
                if desktop_fallback {
                    eprintln!(
                        "watch_friend: desktop fallback enabled target=\"{}\"",
                        targets[0].meta.target
                    );
                } else {
                    eprintln!(
                        "watch_friend: desktop fallback disabled for multiple targets targets={}",
                        targets.len()
                    );
                }
            }

            run_watch_targets(
                targets,
                WatchWorkerOptions {
                    save_dir,
                    interval_sec: args.interval_sec,
                    once: args.once,
                    verbose: args.verbose,
                    desktop_fallback,
                },
                args.max_events,
                args.idle_timeout_sec,
            )
            .await
        })
    }
}

async fn prepare_watch_targets(
    raw_targets: Vec<String>,
    save_dir: &Path,
    state_file: Option<PathBuf>,
    verbose: bool,
) -> Result<Vec<WatchTarget>> {
    if raw_targets.is_empty() {
        bail!("至少需要一个 --to");
    }
    if raw_targets.len() > 1 && state_file.is_some() {
        bail!("--state-file 仅支持单个 --to，多目标模式会按 chat_id 自动保存独立状态文件");
    }

    let mut targets = Vec::with_capacity(raw_targets.len());
    let mut seen_targets = BTreeSet::new();
    for raw_target in raw_targets {
        let target = validate_target(&raw_target)?;
        if !seen_targets.insert(target.clone()) {
            bail!("--to 重复: {target}");
        }
        targets.push(target);
    }

    let (begin_time, end_time) = recent_seven_day_window();
    let chats = fetch_chat_list(&begin_time, &end_time).await?;
    let mut contact_users: Option<Vec<ContactUser>> = None;
    let mut seen_chat_ids = BTreeSet::new();
    let mut prepared = Vec::with_capacity(targets.len());

    for target in targets {
        let chat = match select_chat_from_recent(&target, &chats)? {
            Some(chat) => chat,
            None => {
                if contact_users.is_none() {
                    contact_users = Some(fetch_contact_userlist().await?);
                }
                let users = contact_users.as_deref().unwrap_or_default();
                match select_contact_user(&target, users)? {
                    Some(user) => contact_user_to_chat_summary(user),
                    None => bail!("未在最近 7 天会话列表或通讯录中找到好友: {target}"),
                }
            }
        };

        if !seen_chat_ids.insert(chat.chat_id.clone()) {
            bail!(
                "多个 --to 解析到同一 chat_id: {} target=\"{}\"",
                chat.chat_id,
                target
            );
        }

        if verbose {
            eprintln!(
                "watch_friend: resolved target=\"{}\" chat_id=\"{}\" chat_name=\"{}\" recent_chats={}",
                target,
                chat.chat_id,
                chat.chat_name,
                chats.len()
            );
        }

        let current_state_file = state_file
            .clone()
            .unwrap_or_else(|| state_file_path(save_dir, &chat.chat_id));
        let state = load_state(&current_state_file).await?;
        prepared.push(WatchTarget {
            meta: WatchTargetMeta {
                target,
                chat_id: chat.chat_id,
                chat_name: chat.chat_name,
            },
            state_file: current_state_file,
            state,
        });
    }

    Ok(prepared)
}

async fn run_watch_targets(
    targets: Vec<WatchTarget>,
    options: WatchWorkerOptions,
    max_events: usize,
    idle_timeout_sec: u64,
) -> Result<()> {
    let (event_tx, event_rx) = mpsc::channel(100);
    let (stop_tx, stop_rx) = watch::channel(false);
    let mut handles = Vec::with_capacity(targets.len());

    for target in targets {
        handles.push(spawn_watch_target_worker(
            target,
            options.clone(),
            event_tx.clone(),
            stop_rx.clone(),
        ));
    }
    drop(event_tx);

    let writer_result = write_watch_events(
        event_rx,
        stop_tx.clone(),
        max_events,
        idle_timeout_sec,
        options.verbose,
    )
    .await;
    if writer_result.is_err() {
        let _ = stop_tx.send(true);
    }

    let worker_result = await_watch_workers(handles).await;
    writer_result?;
    worker_result
}

fn spawn_watch_target_worker(
    target: WatchTarget,
    options: WatchWorkerOptions,
    event_tx: mpsc::Sender<WatchEvent>,
    stop_rx: watch::Receiver<bool>,
) -> JoinHandle<Result<()>> {
    tokio::spawn(async move { watch_target_worker(target, options, event_tx, stop_rx).await })
}

async fn watch_target_worker(
    mut target: WatchTarget,
    options: WatchWorkerOptions,
    event_tx: mpsc::Sender<WatchEvent>,
    mut stop_rx: watch::Receiver<bool>,
) -> Result<()> {
    loop {
        if *stop_rx.borrow() {
            save_state_or_log(&target, &event_tx).await;
            return Ok(());
        }

        let (poll_begin_time, poll_end_time) = recent_seven_day_window();
        let mut messages = match fetch_messages(
            &target.meta.chat_id,
            &poll_begin_time,
            &poll_end_time,
        )
        .await
        {
            Ok(messages) => messages,
            Err(err) => {
                if !send_log(
                        &event_tx,
                        format!(
                            "watch_friend: fetch failed target=\"{}\" chat_id=\"{}\" chat_name=\"{}\" error={:#}",
                            target.meta.target, target.meta.chat_id, target.meta.chat_name, err
                        ),
                    )
                    .await
                    {
                        return Ok(());
                    }
                save_state_or_log(&target, &event_tx).await;
                if options.once {
                    return Ok(());
                }
                wait_next_poll(options.interval_sec, &mut stop_rx).await;
                continue;
            }
        };

        if messages.is_empty() && options.desktop_fallback {
            let desktop_messages =
                read_friend_text_messages(&target.meta.target).with_context(|| {
                    format!(
                        "读取桌面消息失败 target=\"{}\" chat_id=\"{}\"",
                        target.meta.target, target.meta.chat_id
                    )
                })?;
            if options.verbose && !desktop_messages.is_empty() {
                send_log(
                    &event_tx,
                    format!(
                        "watch_friend: desktop fallback messages={} target=\"{}\" chat_id=\"{}\"",
                        desktop_messages.len(),
                        target.meta.target,
                        target.meta.chat_id
                    ),
                )
                .await;
            }
            messages = desktop_messages_to_values(desktop_messages);
        }

        if options.verbose {
            send_log(
                &event_tx,
                format!(
                    "watch_friend: fetched messages={} target=\"{}\" chat_id=\"{}\" begin=\"{}\" end=\"{}\"",
                    messages.len(),
                    target.meta.target,
                    target.meta.chat_id,
                    poll_begin_time,
                    poll_end_time
                ),
            )
            .await;
        }

        messages.sort_by_key(message_sort_key);
        for message in messages {
            if *stop_rx.borrow() {
                save_state_or_log(&target, &event_tx).await;
                return Ok(());
            }

            let key = message_dedup_key(&target.meta.chat_id, &message);
            if target.state.seen.contains(&key) {
                continue;
            }

            let output = match build_output_message(&target.meta, message, &options.save_dir).await
            {
                Ok(output) => output,
                Err(err) => {
                    if !send_log(
                        &event_tx,
                        format!(
                            "watch_friend: build output failed target=\"{}\" chat_id=\"{}\" chat_name=\"{}\" error={:#}",
                            target.meta.target,
                            target.meta.chat_id,
                            target.meta.chat_name,
                            err
                        ),
                    )
                    .await
                    {
                        return Ok(());
                    }
                    continue;
                }
            };

            let (ack_tx, ack_rx) = oneshot::channel();
            let event = WatchEvent::Message(WatchOutput {
                output,
                ack: ack_tx,
            });
            if event_tx.send(event).await.is_err() {
                return Ok(());
            }

            match ack_rx.await {
                Ok(true) => {
                    target.state.seen.insert(key);
                    if let Err(err) = save_state(&target.state_file, &target.state) {
                        if !send_log(
                            &event_tx,
                            format!(
                                "watch_friend: save state failed target=\"{}\" chat_id=\"{}\" state_file=\"{}\" error={:#}",
                                target.meta.target,
                                target.meta.chat_id,
                                target.state_file.display(),
                                err
                            ),
                        )
                        .await
                        {
                            return Ok(());
                        }
                    }
                }
                Ok(false) | Err(_) => {
                    save_state_or_log(&target, &event_tx).await;
                    return Ok(());
                }
            }
        }

        save_state_or_log(&target, &event_tx).await;
        if options.once {
            return Ok(());
        }
        wait_next_poll(options.interval_sec, &mut stop_rx).await;
    }
}

async fn write_watch_events(
    mut event_rx: mpsc::Receiver<WatchEvent>,
    stop_tx: watch::Sender<bool>,
    max_events: usize,
    idle_timeout_sec: u64,
    verbose: bool,
) -> Result<()> {
    let mut emitted = 0usize;
    let mut last_emit_at = Instant::now();
    let idle_timeout = StdDuration::from_secs(idle_timeout_sec);

    loop {
        let event = if idle_timeout_sec > 0 {
            let deadline = last_emit_at + idle_timeout;
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => {
                    if verbose {
                        eprintln!(
                            "watch_friend: idle timeout reached after {}s without new messages",
                            idle_timeout_sec
                        );
                    }
                    let _ = stop_tx.send(true);
                    return Ok(());
                }
                event = event_rx.recv() => event,
            }
        } else {
            event_rx.recv().await
        };

        let Some(event) = event else {
            return Ok(());
        };

        match event {
            WatchEvent::Message(message) => {
                println!("{}", serde_json::to_string(&message.output)?);
                emitted += 1;
                last_emit_at = Instant::now();
                let reached_limit = max_events > 0 && emitted >= max_events;
                let _ = message.ack.send(true);
                if reached_limit {
                    let _ = stop_tx.send(true);
                    return Ok(());
                }
            }
            WatchEvent::Log(message) => {
                eprintln!("{message}");
            }
        }
    }
}

async fn wait_next_poll(interval_sec: u64, stop_rx: &mut watch::Receiver<bool>) {
    tokio::select! {
        _ = tokio::time::sleep(StdDuration::from_secs(interval_sec)) => {}
        changed = stop_rx.changed() => {
            let _ = changed;
        }
    }
}

async fn send_log(event_tx: &mpsc::Sender<WatchEvent>, message: String) -> bool {
    event_tx.send(WatchEvent::Log(message)).await.is_ok()
}

async fn save_state_or_log(target: &WatchTarget, event_tx: &mpsc::Sender<WatchEvent>) {
    if let Err(err) = save_state(&target.state_file, &target.state) {
        let _ = send_log(
            event_tx,
            format!(
                "watch_friend: save state failed target=\"{}\" chat_id=\"{}\" state_file=\"{}\" error={:#}",
                target.meta.target,
                target.meta.chat_id,
                target.state_file.display(),
                err
            ),
        )
        .await;
    }
}

async fn await_watch_workers(handles: Vec<JoinHandle<Result<()>>>) -> Result<()> {
    let mut first_error = None;
    for handle in handles {
        match handle.await {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                if first_error.is_none() {
                    first_error = Some(err);
                }
            }
            Err(err) => {
                if first_error.is_none() {
                    first_error = Some(anyhow::anyhow!("watch_friend worker join failed: {err}"));
                }
            }
        }
    }

    match first_error {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

fn message_sort_key(message: &Value) -> String {
    message
        .get("send_time")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn recent_seven_day_window() -> (String, String) {
    let end = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
        + 8 * 60 * 60;
    let begin = end - 7 * 24 * 60 * 60 + 1;
    (format_wecom_time(begin), format_wecom_time(end))
}

fn format_wecom_time(beijing_unix_seconds: i64) -> String {
    let days = beijing_unix_seconds.div_euclid(86_400);
    let seconds_of_day = beijing_unix_seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;

    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
}

fn civil_from_days(days_since_unix_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_unix_epoch + 719_468;
    let adjusted = if z >= 0 { z } else { z - 146_096 };
    let era = adjusted.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if month <= 2 { 1 } else { 0 };

    (year, month, day)
}

async fn fetch_chat_list(begin_time: &str, end_time: &str) -> Result<Vec<ChatSummary>> {
    let mut cursor: Option<String> = None;
    let mut chats = Vec::new();

    for _ in 0..MAX_PAGES {
        let mut args = json!({
            "begin_time": begin_time,
            "end_time": end_time,
        });
        if let Some(cursor) = cursor.as_deref() {
            args["cursor"] = Value::String(cursor.to_string());
        }

        let res = json_rpc::call_json_tool("msg", "get_msg_chat_list", args).await?;
        let page_chats = res
            .get("chats")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("get_msg_chat_list 响应缺少 chats 数组"))?;

        for item in page_chats {
            let chat: ChatSummary =
                serde_json::from_value(item.clone()).context("解析会话列表失败")?;
            if !chat.chat_id.is_empty() {
                chats.push(chat);
            }
        }

        cursor = res
            .get("next_cursor")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string);
        let has_more = res
            .get("has_more")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !has_more || cursor.is_none() {
            return Ok(chats);
        }
    }

    bail!("get_msg_chat_list 分页超过 {MAX_PAGES} 页，已停止以避免无限循环")
}

async fn fetch_contact_userlist() -> Result<Vec<ContactUser>> {
    let res = json_rpc::call_json_tool("contact", "get_userlist", json!({})).await?;
    let users = res
        .get("userlist")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("get_userlist 响应缺少 userlist 数组"))?;

    let mut parsed = Vec::new();
    for item in users {
        let user: ContactUser =
            serde_json::from_value(item.clone()).context("解析通讯录成员列表失败")?;
        if !user.userid.is_empty() {
            parsed.push(user);
        }
    }

    Ok(parsed)
}

fn select_chat_from_recent(target: &str, chats: &[ChatSummary]) -> Result<Option<ChatSummary>> {
    let exact: Vec<_> = chats
        .iter()
        .filter(|chat| chat.chat_id == target || chat.chat_name == target)
        .cloned()
        .collect();
    match exact.len() {
        1 => return Ok(Some(exact[0].clone())),
        n if n > 1 => return Err(ambiguous_candidates_error("找到多个精确匹配会话", exact)),
        _ => {}
    }

    let fuzzy: Vec<_> = chats
        .iter()
        .filter(|chat| chat.chat_name.contains(target))
        .cloned()
        .collect();
    match fuzzy.len() {
        1 => Ok(Some(fuzzy[0].clone())),
        n if n > 1 => Err(ambiguous_candidates_error("找到多个模糊匹配会话", fuzzy)),
        _ => Ok(None),
    }
}

fn select_contact_user(target: &str, users: &[ContactUser]) -> Result<Option<ContactUser>> {
    let exact: Vec<_> = users
        .iter()
        .filter(|user| {
            user.userid == target
                || user.name == target
                || (!user.alias.is_empty() && user.alias == target)
        })
        .cloned()
        .collect();
    match exact.len() {
        1 => return Ok(Some(exact[0].clone())),
        n if n > 1 => return Err(ambiguous_candidates_error("找到多个精确匹配成员", exact)),
        _ => {}
    }

    let fuzzy: Vec<_> = users
        .iter()
        .filter(|user| {
            user.name.contains(target) || (!user.alias.is_empty() && user.alias.contains(target))
        })
        .cloned()
        .collect();
    match fuzzy.len() {
        1 => Ok(Some(fuzzy[0].clone())),
        n if n > 1 => Err(ambiguous_candidates_error("找到多个模糊匹配成员", fuzzy)),
        _ => Ok(None),
    }
}

fn contact_user_to_chat_summary(user: ContactUser) -> ChatSummary {
    let chat_name = if !user.alias.is_empty() {
        user.alias
    } else if !user.name.is_empty() {
        user.name
    } else {
        user.userid.clone()
    };

    ChatSummary {
        chat_id: user.userid,
        chat_name,
        last_msg_time: None,
        msg_count: None,
    }
}

fn ambiguous_candidates_error<T>(reason: &str, candidates: Vec<T>) -> anyhow::Error
where
    T: Serialize,
{
    let payload = json!({
        "error": reason,
        "candidates": candidates,
    });
    anyhow::anyhow!(
        "{}",
        serde_json::to_string(&payload).unwrap_or_else(|_| reason.to_string())
    )
}

async fn fetch_messages(chat_id: &str, begin_time: &str, end_time: &str) -> Result<Vec<Value>> {
    let mut cursor: Option<String> = None;
    let mut messages = Vec::new();

    for _ in 0..MAX_PAGES {
        let mut args = json!({
            "chat_type": 1,
            "chatid": chat_id,
            "begin_time": begin_time,
            "end_time": end_time,
        });
        if let Some(cursor) = cursor.as_deref() {
            args["cursor"] = Value::String(cursor.to_string());
        }

        let res = json_rpc::call_json_tool("msg", "get_message", args).await?;
        let page_messages = res
            .get("messages")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("get_message 响应缺少 messages 数组"))?;
        messages.extend(page_messages.iter().cloned());

        cursor = res
            .get("next_cursor")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string);
        if cursor.is_none() {
            return Ok(messages);
        }
    }

    bail!("get_message 分页超过 {MAX_PAGES} 页，已停止以避免无限循环")
}

async fn build_output_message(
    meta: &WatchTargetMeta,
    message: Value,
    save_dir: &Path,
) -> Result<Value> {
    let msgtype = message
        .get("msgtype")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let mut output = Map::new();
    output.insert("chat_id".to_string(), Value::String(meta.chat_id.clone()));
    output.insert("target".to_string(), Value::String(meta.target.clone()));
    output.insert(
        "chat_name".to_string(),
        Value::String(meta.chat_name.clone()),
    );
    copy_string_field(&mut output, &message, "send_time");
    copy_string_field(&mut output, &message, "userid");
    if let Some(source) = message.pointer("/desktop/source").and_then(Value::as_str) {
        output.insert("source".to_string(), Value::String(source.to_string()));
    }
    output.insert("msgtype".to_string(), Value::String(msgtype.to_string()));

    match msgtype {
        "text" => {
            if let Some(content) = message
                .pointer("/text/content")
                .and_then(Value::as_str)
                .map(ToString::to_string)
            {
                output.insert("text".to_string(), Value::String(content));
            }
        }
        "image" | "file" => {
            append_media_fields(&mut output, &message, msgtype);
            if let Some(media_id) = media_id_for(&message, msgtype) {
                let media_res =
                    json_rpc::call_tool("msg", "get_msg_media", json!({ "media_id": media_id }))
                        .await?;
                if let Some(item) = media::extract_media_item_to_dir(media_res, save_dir).await? {
                    copy_media_item_field(&mut output, &item, "local_path");
                    copy_media_item_field(&mut output, &item, "size");
                    copy_media_item_field(&mut output, &item, "content_type");
                    copy_media_item_field(&mut output, &item, "name");
                }
            }
        }
        "voice" | "video" => {
            append_media_fields(&mut output, &message, msgtype);
            output.insert("saved".to_string(), Value::Bool(false));
        }
        _ => {
            output.insert("raw".to_string(), message);
        }
    }

    Ok(Value::Object(output))
}

fn copy_string_field(output: &mut Map<String, Value>, message: &Value, key: &str) {
    if let Some(value) = message.get(key).and_then(Value::as_str) {
        output.insert(key.to_string(), Value::String(value.to_string()));
    }
}

fn append_media_fields(output: &mut Map<String, Value>, message: &Value, msgtype: &str) {
    if let Some(media_id) = media_id_for(message, msgtype) {
        output.insert("media_id".to_string(), Value::String(media_id.to_string()));
    }
    if let Some(name) = message
        .get(msgtype)
        .and_then(|item| item.get("name"))
        .and_then(Value::as_str)
    {
        output.insert("name".to_string(), Value::String(name.to_string()));
    }
}

fn copy_media_item_field(output: &mut Map<String, Value>, item: &Value, key: &str) {
    if let Some(value) = item.get(key) {
        output.insert(key.to_string(), value.clone());
    }
}

fn media_id_for<'a>(message: &'a Value, msgtype: &str) -> Option<&'a str> {
    message
        .get(msgtype)
        .and_then(|item| item.get("media_id"))
        .and_then(Value::as_str)
}

fn message_dedup_key(chat_id: &str, message: &Value) -> String {
    if let Some(desktop_key) = message.pointer("/desktop/key").and_then(Value::as_str) {
        return format!("{chat_id}|desktop|{desktop_key}");
    }

    let send_time = message
        .get("send_time")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let userid = message
        .get("userid")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let msgtype = message
        .get("msgtype")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let body_key = match msgtype {
        "text" => message
            .pointer("/text/content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        "image" | "file" | "voice" | "video" => media_id_for(message, msgtype)
            .unwrap_or_default()
            .to_string(),
        _ => serde_json::to_string(message).unwrap_or_default(),
    };

    format!("{chat_id}|{send_time}|{userid}|{msgtype}|{body_key}")
}

fn desktop_messages_to_values(messages: Vec<DesktopChatMessage>) -> Vec<Value> {
    messages
        .into_iter()
        .map(|message| {
            json!({
                "userid": "desktop",
                "msgtype": "text",
                "text": {
                    "content": message.text,
                },
                "desktop": {
                    "key": message.key,
                    "source": "macos_accessibility",
                },
            })
        })
        .collect()
}

fn state_file_path(save_dir: &Path, chat_id: &str) -> PathBuf {
    let name = sanitize_filename(chat_id).unwrap_or_else(|| "chat".to_string());
    save_dir.join(format!(".watch_friend_{name}.json"))
}

async fn load_state(path: &Path) -> Result<WatchState> {
    match tokio::fs::read_to_string(path).await {
        Ok(content) => serde_json::from_str(&content)
            .with_context(|| format!("解析状态文件失败: {}", path.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(WatchState::default()),
        Err(err) => Err(err).with_context(|| format!("读取状态文件失败: {}", path.display())),
    }
}

fn save_state(path: &Path, state: &WatchState) -> Result<()> {
    let data = serde_json::to_vec_pretty(state)?;
    atomic_write(path, &data, Some(0o600))
        .with_context(|| format!("写入状态文件失败: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat(id: &str, name: &str) -> ChatSummary {
        ChatSummary {
            chat_id: id.to_string(),
            chat_name: name.to_string(),
            last_msg_time: None,
            msg_count: None,
        }
    }

    fn user(userid: &str, name: &str, alias: &str) -> ContactUser {
        ContactUser {
            userid: userid.to_string(),
            name: name.to_string(),
            alias: alias.to_string(),
        }
    }

    #[test]
    fn watch_friend_args_accepts_repeated_to() {
        let matches = WatchFriendArgs::augment_args(Command::new("test"))
            .try_get_matches_from(["test", "--to", "张三", "--to", "李四"])
            .unwrap();
        let args = WatchFriendArgs::from_arg_matches(&matches).unwrap();

        assert_eq!(args.to, vec!["张三".to_string(), "李四".to_string()]);
    }

    #[test]
    fn watch_friend_args_accepts_single_to() {
        let matches = WatchFriendArgs::augment_args(Command::new("test"))
            .try_get_matches_from(["test", "--to", "张三"])
            .unwrap();
        let args = WatchFriendArgs::from_arg_matches(&matches).unwrap();

        assert_eq!(args.to, vec!["张三".to_string()]);
    }

    #[test]
    fn select_chat_prefers_unique_exact_match() {
        let chats = vec![chat("u1", "张三"), chat("u2", "张三丰")];
        let selected = select_chat_from_recent("张三", &chats).unwrap().unwrap();
        assert_eq!(selected.chat_id, "u1");
    }

    #[test]
    fn select_chat_rejects_ambiguous_fuzzy_matches() {
        let chats = vec![chat("u1", "张三"), chat("u2", "张三丰")];
        let err = select_chat_from_recent("张", &chats)
            .unwrap_err()
            .to_string();
        assert!(err.contains("candidates"));
    }

    #[test]
    fn select_chat_from_recent_returns_none_for_missing_chat() {
        let chats = vec![chat("u1", "张三")];
        let selected = select_chat_from_recent("邹友", &chats).unwrap();
        assert!(selected.is_none());
    }

    #[test]
    fn select_contact_user_matches_name_and_alias() {
        let users = vec![user("u1", "张三", ""), user("u2", "邹友", "客户-邹友")];

        let by_name = select_contact_user("邹友", &users).unwrap().unwrap();
        assert_eq!(by_name.userid, "u2");

        let by_alias = select_contact_user("客户-邹友", &users).unwrap().unwrap();
        assert_eq!(by_alias.userid, "u2");
    }

    #[test]
    fn select_contact_user_rejects_ambiguous_fuzzy_matches() {
        let users = vec![user("u1", "邹友", ""), user("u2", "邹友明", "")];
        let err = select_contact_user("邹", &users).unwrap_err().to_string();
        assert!(err.contains("candidates"));
    }

    #[test]
    fn contact_user_to_chat_summary_uses_userid_as_chat_id() {
        let chat = contact_user_to_chat_summary(user("u2", "邹友", ""));
        assert_eq!(chat.chat_id, "u2");
        assert_eq!(chat.chat_name, "邹友");
    }

    #[test]
    fn message_dedup_key_uses_media_id_for_file_messages() {
        let message = json!({
            "userid": "lisi",
            "send_time": "2026-03-17 09:40:00",
            "msgtype": "file",
            "file": {
                "media_id": "MEDIAID_yyyyyy",
                "name": "report.pdf"
            }
        });

        assert_eq!(
            message_dedup_key("chat1", &message),
            "chat1|2026-03-17 09:40:00|lisi|file|MEDIAID_yyyyyy"
        );
    }

    #[test]
    fn message_dedup_key_uses_desktop_key_when_available() {
        let message = json!({
            "userid": "desktop",
            "msgtype": "text",
            "text": {
                "content": "你好"
            },
            "desktop": {
                "key": "1|341, 42|20, 22"
            }
        });

        assert_eq!(
            message_dedup_key("chat1", &message),
            "chat1|desktop|1|341, 42|20, 22"
        );
    }

    #[test]
    fn desktop_messages_to_values_preserves_text_and_key() {
        let values = desktop_messages_to_values(vec![DesktopChatMessage {
            key: "1|341, 42|20, 22".to_string(),
            text: "你好".to_string(),
        }]);

        assert_eq!(
            values[0].pointer("/text/content").and_then(Value::as_str),
            Some("你好")
        );
        assert_eq!(
            values[0].pointer("/desktop/key").and_then(Value::as_str),
            Some("1|341, 42|20, 22")
        );
    }

    #[tokio::test]
    async fn build_output_message_adds_target_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let meta = WatchTargetMeta {
            target: "张三-客户".to_string(),
            chat_id: "zhangsan".to_string(),
            chat_name: "张三".to_string(),
        };
        let output = build_output_message(
            &meta,
            json!({
                "userid": "lisi",
                "send_time": "2026-03-17 09:40:00",
                "msgtype": "text",
                "text": {
                    "content": "你好"
                }
            }),
            dir.path(),
        )
        .await
        .unwrap();

        assert_eq!(
            output.get("target").and_then(Value::as_str),
            Some("张三-客户")
        );
        assert_eq!(
            output.get("chat_id").and_then(Value::as_str),
            Some("zhangsan")
        );
        assert_eq!(
            output.get("chat_name").and_then(Value::as_str),
            Some("张三")
        );
        assert_eq!(output.get("text").and_then(Value::as_str), Some("你好"));
    }

    #[test]
    fn recent_window_stays_within_seven_days() {
        let (begin, end) = recent_seven_day_window();
        assert!(begin < end);
        assert_eq!(begin.len(), "2026-03-17 09:40:00".len());
        assert_eq!(end.len(), "2026-03-17 09:40:00".len());
    }

    #[test]
    fn format_wecom_time_uses_beijing_offset_seconds() {
        assert_eq!(format_wecom_time(8 * 60 * 60), "1970-01-01 08:00:00");
        assert_eq!(format_wecom_time(0), "1970-01-01 00:00:00");
    }

    #[tokio::test]
    async fn state_file_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut state = WatchState::default();
        state.seen.insert("key1".to_string());

        save_state(&path, &state).unwrap();
        let loaded = load_state(&path).await.unwrap();
        assert!(loaded.seen.contains("key1"));
    }
}
