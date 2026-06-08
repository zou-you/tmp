use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Duration as StdDuration;

use anyhow::{Context, Result, bail};
use clap::{ArgMatches, Args, Command, FromArgMatches};
use serde_json::{Value, json};
use tokio::time::Instant;

use crate::helpers::msg::watch_friend::{
    ChatSummary, WatchState, WatchTargetMeta, build_output_message, fetch_chat_list,
    fetch_messages, load_state, message_dedup_key, message_sort_key, recent_seconds_window,
    save_state,
};
use crate::helpers::registry::Helper;
use crate::helpers::wecom_desktop::{
    DesktopChatMessage, DesktopRecentChatTarget, read_friend_text_messages,
    read_recent_chat_target_summaries, validate_target,
};
use crate::paths;

const DESKTOP_RECENT_CHAT_LIMIT: usize = 12;
const DESKTOP_FULL_SWEEP_SECONDS: u64 = 60;
const DESKTOP_MISSING_QUEUE_SCAN_PER_POLL: usize = 1;

#[derive(Args, Debug)]
pub struct WatchAllArgs {
    /// 轮询间隔秒数
    #[arg(long, default_value_t = 10)]
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

    /// 初始监听队列用户；指定后默认写入 save-dir/.watch_all_queue.txt，可运行中编辑该文件增删用户
    #[arg(long, num_args = 1, action = clap::ArgAction::Append)]
    pub to: Vec<String>,

    /// 动态监听队列文件；支持一行一个用户名，或 JSON 字符串数组
    #[arg(long)]
    pub queue_file: Option<PathBuf>,

    /// 队列模式下仍启用服务端消息接口轮询；默认队列模式只走桌面监听以降低延迟
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub include_server: bool,

    /// 打印轮询诊断信息到 stderr；stdout 仍只输出消息 NDJSON
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub verbose: bool,

    /// 自定义状态文件路径，默认写入 save-dir 下的 .watch_all.json
    #[arg(long, hide = true)]
    pub state_file: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct WatchAllOptions {
    save_dir: PathBuf,
    interval_sec: u64,
    once: bool,
    max_events: usize,
    idle_timeout_sec: u64,
    verbose: bool,
    queue_file: Option<PathBuf>,
    queue_targets: Option<BTreeSet<String>>,
    include_server: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChatPollSignature {
    last_msg_time: Option<String>,
    msg_count: Option<u64>,
}

#[derive(Debug)]
struct PendingOutput {
    key: String,
    output: Value,
}

#[derive(Debug)]
struct WatchAllRuntime {
    state: WatchState,
    chat_signatures: BTreeMap<String, ChatPollSignature>,
    desktop_signatures: BTreeMap<String, String>,
    desktop_snapshots: BTreeMap<String, Vec<String>>,
    desktop_initial_previews: BTreeMap<String, Option<String>>,
    desktop_pending_baseline_targets: BTreeSet<String>,
    desktop_missing_scan_cursor: usize,
    pending_baseline_chat_ids: Option<BTreeSet<String>>,
    queue_targets: Option<BTreeSet<String>>,
}

impl WatchAllRuntime {
    fn new(state: WatchState, queue_targets: Option<BTreeSet<String>>) -> Self {
        Self {
            state,
            chat_signatures: BTreeMap::new(),
            desktop_signatures: BTreeMap::new(),
            desktop_snapshots: BTreeMap::new(),
            desktop_initial_previews: BTreeMap::new(),
            desktop_pending_baseline_targets: BTreeSet::new(),
            desktop_missing_scan_cursor: 0,
            pending_baseline_chat_ids: None,
            queue_targets,
        }
    }
}

#[derive(Debug)]
struct PollOutputBuffer {
    max_outputs: usize,
    outputs: Vec<PendingOutput>,
    pending_keys: BTreeSet<String>,
}

impl PollOutputBuffer {
    fn new(max_outputs: usize) -> Self {
        Self {
            max_outputs,
            outputs: Vec::new(),
            pending_keys: BTreeSet::new(),
        }
    }

    fn is_full(&self) -> bool {
        self.outputs.len() >= self.max_outputs
    }
}

pub struct WatchAllHelper;

impl Helper for WatchAllHelper {
    fn category(&self) -> &'static str {
        "msg"
    }

    fn command(&self) -> clap::Command {
        WatchAllArgs::augment_args(
            Command::new("+watch_all").about("准实时轮询所有最近活跃单聊新消息，并保存图片和文件"),
        )
    }

    fn execute<'a>(
        &'a self,
        matches: &'a ArgMatches,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async {
            let args = WatchAllArgs::from_arg_matches(matches)?;
            validate_watch_all_args(&args)?;

            let save_dir = args.save_dir.unwrap_or_else(paths::media_dir);
            tokio::fs::create_dir_all(&save_dir)
                .await
                .with_context(|| format!("创建保存目录失败: {}", save_dir.display()))?;

            let initial_queue_targets = parse_queue_targets(args.to)?;
            let queue_file = args.queue_file.or_else(|| {
                (!initial_queue_targets.is_empty()).then(|| watch_all_queue_file(&save_dir))
            });
            let queue_targets = prepare_watch_queue(&queue_file, &initial_queue_targets).await?;
            let include_server = args.include_server || queue_file.is_none();

            let state_file = args
                .state_file
                .unwrap_or_else(|| watch_all_state_file(&save_dir));
            let state = load_state(&state_file).await?;
            run_watch_all(
                state_file,
                state,
                WatchAllOptions {
                    save_dir,
                    interval_sec: args.interval_sec,
                    once: args.once,
                    max_events: args.max_events,
                    idle_timeout_sec: args.idle_timeout_sec,
                    verbose: args.verbose,
                    queue_file,
                    queue_targets,
                    include_server,
                },
            )
            .await
        })
    }
}

fn validate_watch_all_args(args: &WatchAllArgs) -> Result<()> {
    if args.interval_sec == 0 {
        bail!("--interval-sec 必须大于 0");
    }
    Ok(())
}

fn parse_queue_targets(raw_targets: Vec<String>) -> Result<BTreeSet<String>> {
    let mut targets = BTreeSet::new();
    for raw_target in raw_targets {
        targets.insert(validate_target(&raw_target)?);
    }
    Ok(targets)
}

async fn prepare_watch_queue(
    queue_file: &Option<PathBuf>,
    initial_targets: &BTreeSet<String>,
) -> Result<Option<BTreeSet<String>>> {
    let Some(queue_file) = queue_file else {
        return if initial_targets.is_empty() {
            Ok(None)
        } else {
            Ok(Some(initial_targets.clone()))
        };
    };

    ensure_watch_queue_file(queue_file, initial_targets).await?;
    Ok(Some(load_watch_queue_file(queue_file).await?))
}

async fn ensure_watch_queue_file(path: &Path, initial_targets: &BTreeSet<String>) -> Result<()> {
    if tokio::fs::try_exists(path)
        .await
        .with_context(|| format!("检查监听队列文件失败: {}", path.display()))?
    {
        if initial_targets.is_empty() {
            return Ok(());
        }

        let mut targets = load_watch_queue_file(path).await?;
        let old_len = targets.len();
        targets.extend(initial_targets.iter().cloned());
        if targets.len() != old_len {
            write_watch_queue_file(path, &targets).await?;
        }
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("创建监听队列目录失败: {}", parent.display()))?;
    }
    write_watch_queue_file(path, initial_targets).await
}

async fn write_watch_queue_file(path: &Path, targets: &BTreeSet<String>) -> Result<()> {
    let content = targets.iter().cloned().collect::<Vec<_>>().join("\n");
    let content = if content.is_empty() {
        String::new()
    } else {
        format!("{content}\n")
    };
    tokio::fs::write(path, content)
        .await
        .with_context(|| format!("写入监听队列文件失败: {}", path.display()))
}

async fn load_watch_queue_file(path: &Path) -> Result<BTreeSet<String>> {
    let content = match tokio::fs::read_to_string(path).await {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => {
            return Err(err).with_context(|| format!("读取监听队列文件失败: {}", path.display()));
        }
    };

    parse_watch_queue_content(&content)
        .with_context(|| format!("解析监听队列文件失败: {}", path.display()))
}

fn parse_watch_queue_content(content: &str) -> Result<BTreeSet<String>> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Ok(BTreeSet::new());
    }

    if trimmed.starts_with('[') {
        let targets = serde_json::from_str::<Vec<String>>(trimmed)?;
        return parse_queue_targets(targets);
    }

    let targets = content
        .lines()
        .filter_map(|line| {
            let line = line.split_once('#').map_or(line, |(head, _)| head).trim();
            (!line.is_empty()).then(|| line.to_string())
        })
        .collect::<Vec<_>>();
    parse_queue_targets(targets)
}

async fn refresh_watch_queue(
    options: &WatchAllOptions,
    runtime: &mut WatchAllRuntime,
    poll_index: u64,
) {
    let Some(queue_file) = options.queue_file.as_deref() else {
        return;
    };

    let next_targets = match load_watch_queue_file(queue_file).await {
        Ok(targets) => targets,
        Err(err) => {
            if options.verbose {
                eprintln!(
                    "watch_all: queue reload failed file=\"{}\" error={:#}",
                    queue_file.display(),
                    err
                );
            }
            return;
        }
    };

    let old_targets = runtime.queue_targets.clone().unwrap_or_default();
    if old_targets == next_targets {
        return;
    }

    for removed in old_targets.difference(&next_targets) {
        remove_desktop_target_state(removed, runtime);
    }

    if poll_index > 0 {
        runtime
            .desktop_pending_baseline_targets
            .extend(next_targets.difference(&old_targets).cloned());
    }

    if options.verbose {
        eprintln!(
            "watch_all: queue targets={} names={}",
            next_targets.len(),
            next_targets.iter().cloned().collect::<Vec<_>>().join(",")
        );
    }
    runtime.queue_targets = Some(next_targets);
}

fn remove_desktop_target_state(target: &str, runtime: &mut WatchAllRuntime) {
    let chat_id = desktop_chat_id(target);
    runtime.desktop_pending_baseline_targets.remove(target);
    runtime.desktop_signatures.remove(&chat_id);
    runtime.desktop_snapshots.remove(&chat_id);
    runtime.desktop_initial_previews.remove(&chat_id);
}

async fn run_watch_all(
    state_file: PathBuf,
    state: WatchState,
    options: WatchAllOptions,
) -> Result<()> {
    let mut runtime = WatchAllRuntime::new(state, options.queue_targets.clone());
    let mut emitted = 0usize;
    let mut last_emit_at = Instant::now();
    let mut poll_index = 0u64;

    loop {
        let poll_started_at = Instant::now();
        if options.max_events > 0 && emitted >= options.max_events {
            return Ok(());
        }

        let max_outputs = if options.max_events == 0 {
            usize::MAX
        } else {
            options.max_events - emitted
        };
        refresh_watch_queue(&options, &mut runtime, poll_index).await;
        let outputs = poll_watch_all_once(
            &mut runtime,
            poll_index == 0,
            should_force_desktop_sweep(poll_index, options.interval_sec),
            &options,
            max_outputs,
        )
        .await?;
        poll_index = poll_index.saturating_add(1);

        for pending in outputs {
            if options.verbose {
                eprintln!("watch_all: emit at=\"{}\"", watch_all_now_time());
            }
            println!("{}", serde_json::to_string(&pending.output)?);
            io::stdout().flush()?;
            runtime.state.seen.insert(pending.key);
            emitted += 1;
            last_emit_at = Instant::now();
            save_state_or_log(&state_file, &runtime.state);

            if options.max_events > 0 && emitted >= options.max_events {
                return Ok(());
            }
        }

        save_state_or_log(&state_file, &runtime.state);
        if options.once {
            return Ok(());
        }

        let next_poll_at = poll_started_at + StdDuration::from_secs(options.interval_sec);
        if wait_next_poll_or_idle(&options, last_emit_at, next_poll_at).await {
            return Ok(());
        }
    }
}

async fn poll_watch_all_once(
    runtime: &mut WatchAllRuntime,
    baseline_desktop_targets: bool,
    force_desktop_sweep: bool,
    options: &WatchAllOptions,
    max_outputs: usize,
) -> Result<Vec<PendingOutput>> {
    let (begin_time, end_time) = watch_all_poll_window(options.interval_sec);
    let chats = if options.include_server {
        match fetch_chat_list(&begin_time, &end_time).await {
            Ok(chats) => filter_chats_by_queue(chats, runtime.queue_targets.as_ref()),
            Err(err) => {
                eprintln!(
                    "watch_all: fetch chat list failed begin=\"{}\" end=\"{}\" error={:#}",
                    begin_time, end_time, err
                );
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    let desktop_targets =
        fetch_desktop_recent_targets(options.verbose, runtime.queue_targets.as_ref());
    let desktop_target_names = desktop_targets
        .iter()
        .map(|target| target.name.clone())
        .collect::<BTreeSet<_>>();

    if runtime.pending_baseline_chat_ids.is_none() {
        let baseline_ids = chats
            .iter()
            .map(|chat| chat.chat_id.clone())
            .collect::<BTreeSet<_>>();
        if options.verbose {
            eprintln!(
                "watch_all: baseline active_chats={} desktop_chats={} begin=\"{}\" end=\"{}\"",
                chats.len(),
                desktop_targets.len(),
                begin_time,
                end_time
            );
        }
        runtime.pending_baseline_chat_ids = Some(baseline_ids);
    } else if options.verbose {
        eprintln!(
            "watch_all: active_chats={} desktop_chats={} begin=\"{}\" end=\"{}\"",
            chats.len(),
            desktop_targets.len(),
            begin_time,
            end_time
        );
    }

    let mut buffer = PollOutputBuffer::new(max_outputs);
    for chat in chats {
        if buffer.is_full() {
            break;
        }

        let baseline_chat = runtime
            .pending_baseline_chat_ids
            .as_ref()
            .is_some_and(|ids| ids.contains(&chat.chat_id));
        if !baseline_chat && !chat_needs_fetch(&chat, &runtime.chat_signatures) {
            continue;
        }

        let mut messages = match fetch_messages(&chat.chat_id, &begin_time, &end_time).await {
            Ok(messages) => messages,
            Err(err) => {
                if options.verbose {
                    eprintln!(
                        "watch_all: fetch failed chat_id=\"{}\" chat_name=\"{}\" error={:#}",
                        chat.chat_id, chat.chat_name, err
                    );
                }
                continue;
            }
        };
        messages.sort_by_key(message_sort_key);

        if options.verbose {
            eprintln!(
                "watch_all: fetched messages={} chat_id=\"{}\" chat_name=\"{}\" baseline={}",
                messages.len(),
                chat.chat_id,
                chat.chat_name,
                baseline_chat
            );
        }

        let meta = watch_target_meta_from_chat(&chat);
        for message in messages {
            if buffer.is_full() {
                break;
            }

            let Some(key) = take_unseen_message_key(
                &mut runtime.state,
                &chat.chat_id,
                &message,
                baseline_chat,
                &mut buffer.pending_keys,
            ) else {
                continue;
            };

            let output = match build_output_message(&meta, message, &options.save_dir).await {
                Ok(output) => output,
                Err(err) => {
                    if options.verbose {
                        eprintln!(
                            "watch_all: build output failed chat_id=\"{}\" chat_name=\"{}\" error={:#}",
                            chat.chat_id, chat.chat_name, err
                        );
                    }
                    buffer.pending_keys.remove(&key);
                    continue;
                }
            };
            buffer.outputs.push(PendingOutput { key, output });
        }

        if let Some(ids) = runtime.pending_baseline_chat_ids.as_mut() {
            ids.remove(&chat.chat_id);
        }
        update_chat_signature(&mut runtime.chat_signatures, &chat);
    }

    poll_desktop_recent_targets(
        &desktop_targets,
        runtime,
        baseline_desktop_targets,
        force_desktop_sweep,
        options,
        &mut buffer,
    )
    .await;

    poll_missing_queue_targets(&desktop_target_names, runtime, options, &mut buffer).await;

    Ok(buffer.outputs)
}

async fn poll_desktop_recent_targets(
    targets: &[DesktopRecentChatTarget],
    runtime: &mut WatchAllRuntime,
    baseline_desktop_targets: bool,
    force_desktop_sweep: bool,
    options: &WatchAllOptions,
    buffer: &mut PollOutputBuffer,
) {
    if baseline_desktop_targets {
        for target in targets {
            let chat_id = desktop_chat_id(&target.name);
            runtime
                .desktop_signatures
                .insert(chat_id.clone(), target.signature.clone());
            runtime
                .desktop_initial_previews
                .insert(chat_id, target.preview.clone());
            runtime
                .desktop_pending_baseline_targets
                .remove(&target.name);
        }
        return;
    }

    baseline_pending_desktop_targets(
        targets,
        &mut runtime.desktop_pending_baseline_targets,
        &mut runtime.desktop_signatures,
        &mut runtime.desktop_initial_previews,
    );

    let mut poll_targets = targets
        .iter()
        .enumerate()
        .filter_map(|(index, target)| {
            let chat_id = desktop_chat_id(&target.name);
            let changed = runtime
                .desktop_signatures
                .get(&chat_id)
                .is_none_or(|signature| signature != &target.signature);
            let has_snapshot = runtime.desktop_snapshots.contains_key(&chat_id);
            let has_initial_preview = runtime
                .desktop_initial_previews
                .get(&chat_id)
                .and_then(Option::as_ref)
                .is_some();
            let priority = if changed {
                0
            } else if force_desktop_sweep && (has_snapshot || has_initial_preview) {
                1
            } else {
                return None;
            };

            Some((priority, index, target))
        })
        .collect::<Vec<_>>();
    poll_targets.sort_by_key(|(priority, index, _)| (*priority, *index));

    for (_, _, target) in poll_targets {
        if buffer.is_full() {
            break;
        }

        let chat_id = desktop_chat_id(&target.name);
        let changed = runtime
            .desktop_signatures
            .get(&chat_id)
            .is_none_or(|signature| signature != &target.signature);
        if changed
            && !force_desktop_sweep
            && push_desktop_preview_output(target, &chat_id, runtime, options, buffer).await
        {
            runtime
                .desktop_signatures
                .insert(chat_id, target.signature.clone());
            continue;
        }

        let desktop_messages = match read_friend_text_messages(&target.name) {
            Ok(messages) => messages,
            Err(err) => {
                if options.verbose {
                    eprintln!(
                        "watch_all: desktop fetch failed target=\"{}\" error={:#}",
                        target.name, err
                    );
                }
                continue;
            }
        };
        let message_texts = desktop_message_texts(&desktop_messages);
        let new_message_start = desktop_new_message_start(
            runtime.desktop_snapshots.get(&chat_id),
            &message_texts,
            runtime
                .desktop_initial_previews
                .get(&chat_id)
                .and_then(Option::as_deref),
        );
        let messages = desktop_messages_to_values(desktop_messages);

        if options.verbose {
            eprintln!(
                "watch_all: desktop messages={} target=\"{}\" changed={} force_sweep={} new_start={}",
                messages.len(),
                target.name,
                runtime
                    .desktop_signatures
                    .get(&chat_id)
                    .is_none_or(|signature| signature != &target.signature),
                force_desktop_sweep,
                new_message_start
            );
        }

        let meta = WatchTargetMeta {
            target: target.name.clone(),
            chat_id: chat_id.clone(),
            chat_name: target.name.clone(),
        };
        for message in messages.into_iter().skip(new_message_start) {
            if buffer.is_full() {
                break;
            }

            if desktop_message_was_emitted_from_preview(&chat_id, &message, &runtime.state) {
                continue;
            }

            let key = message_dedup_key(&chat_id, &message);
            if runtime.state.seen.contains(&key) || buffer.pending_keys.contains(&key) {
                continue;
            }
            buffer.pending_keys.insert(key.clone());

            let output = match build_output_message(&meta, message, &options.save_dir).await {
                Ok(output) => output,
                Err(err) => {
                    if options.verbose {
                        eprintln!(
                            "watch_all: desktop build output failed target=\"{}\" error={:#}",
                            target.name, err
                        );
                    }
                    buffer.pending_keys.remove(&key);
                    continue;
                }
            };
            buffer.outputs.push(PendingOutput { key, output });
        }
        runtime
            .desktop_signatures
            .insert(chat_id.clone(), target.signature.clone());
        runtime.desktop_snapshots.insert(chat_id, message_texts);
    }
}

async fn push_desktop_preview_output(
    target: &DesktopRecentChatTarget,
    chat_id: &str,
    runtime: &mut WatchAllRuntime,
    options: &WatchAllOptions,
    buffer: &mut PollOutputBuffer,
) -> bool {
    let Some(preview) = target
        .preview
        .as_deref()
        .filter(|preview| !preview.is_empty())
    else {
        return false;
    };

    let message = desktop_preview_to_value(preview);
    let key = message_dedup_key(chat_id, &message);
    if runtime.state.seen.contains(&key) || buffer.pending_keys.contains(&key) {
        return false;
    }
    buffer.pending_keys.insert(key.clone());

    let meta = WatchTargetMeta {
        target: target.name.clone(),
        chat_id: chat_id.to_string(),
        chat_name: target.name.clone(),
    };
    let output = match build_output_message(&meta, message, &options.save_dir).await {
        Ok(output) => output,
        Err(err) => {
            if options.verbose {
                eprintln!(
                    "watch_all: desktop preview build output failed target=\"{}\" error={:#}",
                    target.name, err
                );
            }
            buffer.pending_keys.remove(&key);
            return false;
        }
    };

    if options.verbose {
        eprintln!(
            "watch_all: desktop preview target=\"{}\" text={}",
            target.name,
            serde_json::to_string(preview).unwrap_or_else(|_| "\"\"".to_string())
        );
    }
    buffer.outputs.push(PendingOutput { key, output });
    true
}

async fn poll_missing_queue_targets(
    visible_targets: &BTreeSet<String>,
    runtime: &mut WatchAllRuntime,
    options: &WatchAllOptions,
    buffer: &mut PollOutputBuffer,
) {
    if buffer.is_full() {
        return;
    }

    let missing_targets = select_missing_queue_targets(
        runtime.queue_targets.as_ref(),
        visible_targets,
        runtime.desktop_missing_scan_cursor,
    );
    if missing_targets.is_empty() {
        runtime.desktop_missing_scan_cursor = 0;
        return;
    }

    let scan_targets = missing_targets
        .iter()
        .take(DESKTOP_MISSING_QUEUE_SCAN_PER_POLL)
        .cloned()
        .collect::<Vec<_>>();
    runtime.desktop_missing_scan_cursor =
        (runtime.desktop_missing_scan_cursor + scan_targets.len()) % missing_targets.len();

    if options.verbose {
        eprintln!(
            "watch_all: queue missing visible={} probe={} names={}",
            missing_targets.len(),
            scan_targets.len(),
            scan_targets.join(",")
        );
    }

    for target in scan_targets {
        if buffer.is_full() {
            break;
        }

        poll_direct_queue_target(&target, runtime, options, buffer).await;
    }
}

async fn poll_direct_queue_target(
    target: &str,
    runtime: &mut WatchAllRuntime,
    options: &WatchAllOptions,
    buffer: &mut PollOutputBuffer,
) {
    let chat_id = desktop_chat_id(target);
    let had_snapshot = runtime.desktop_snapshots.contains_key(&chat_id);
    let pending_baseline = runtime.desktop_pending_baseline_targets.remove(target);
    let needs_baseline = !had_snapshot || pending_baseline;
    let desktop_messages = match read_friend_text_messages(target) {
        Ok(messages) => messages,
        Err(err) => {
            if options.verbose {
                eprintln!(
                    "watch_all: direct queue fetch failed target=\"{}\" error={:#}",
                    target, err
                );
            }
            return;
        }
    };

    let message_texts = desktop_message_texts(&desktop_messages);
    let new_message_start = if needs_baseline {
        message_texts.len()
    } else {
        desktop_new_message_start(
            runtime.desktop_snapshots.get(&chat_id),
            &message_texts,
            runtime
                .desktop_initial_previews
                .get(&chat_id)
                .and_then(Option::as_deref),
        )
    };
    let messages = desktop_messages_to_values(desktop_messages);

    if options.verbose {
        eprintln!(
            "watch_all: direct queue messages={} target=\"{}\" baseline={} new_start={}",
            messages.len(),
            target,
            needs_baseline,
            new_message_start
        );
    }

    if !needs_baseline {
        let meta = WatchTargetMeta {
            target: target.to_string(),
            chat_id: chat_id.clone(),
            chat_name: target.to_string(),
        };
        for message in messages.into_iter().skip(new_message_start) {
            if buffer.is_full() {
                break;
            }

            let key = message_dedup_key(&chat_id, &message);
            if runtime.state.seen.contains(&key) || buffer.pending_keys.contains(&key) {
                continue;
            }
            buffer.pending_keys.insert(key.clone());

            let output = match build_output_message(&meta, message, &options.save_dir).await {
                Ok(output) => output,
                Err(err) => {
                    if options.verbose {
                        eprintln!(
                            "watch_all: direct queue build output failed target=\"{}\" error={:#}",
                            target, err
                        );
                    }
                    buffer.pending_keys.remove(&key);
                    continue;
                }
            };
            buffer.outputs.push(PendingOutput { key, output });
        }
    }

    runtime
        .desktop_snapshots
        .insert(chat_id.clone(), message_texts);
    runtime.desktop_initial_previews.insert(chat_id, None);
}

fn select_missing_queue_targets(
    queue_targets: Option<&BTreeSet<String>>,
    visible_targets: &BTreeSet<String>,
    cursor: usize,
) -> Vec<String> {
    let Some(queue_targets) = queue_targets else {
        return Vec::new();
    };

    let mut missing_targets = queue_targets
        .difference(visible_targets)
        .cloned()
        .collect::<Vec<_>>();
    if missing_targets.is_empty() {
        return missing_targets;
    }

    let missing_len = missing_targets.len();
    missing_targets.rotate_left(cursor % missing_len);
    missing_targets
}

fn baseline_pending_desktop_targets(
    targets: &[DesktopRecentChatTarget],
    desktop_pending_baseline_targets: &mut BTreeSet<String>,
    desktop_signatures: &mut BTreeMap<String, String>,
    desktop_initial_previews: &mut BTreeMap<String, Option<String>>,
) {
    for target in targets {
        if !desktop_pending_baseline_targets.remove(&target.name) {
            continue;
        }

        let chat_id = desktop_chat_id(&target.name);
        desktop_signatures.insert(chat_id.clone(), target.signature.clone());
        desktop_initial_previews.insert(chat_id, target.preview.clone());
    }
}

fn fetch_desktop_recent_targets(
    verbose: bool,
    queue_targets: Option<&BTreeSet<String>>,
) -> Vec<DesktopRecentChatTarget> {
    let limit = desktop_recent_chat_limit(queue_targets);
    match read_recent_chat_target_summaries(limit) {
        Ok(mut targets) => {
            if let Some(queue_targets) = queue_targets {
                targets.retain(|target| queue_targets.contains(&target.name));
            }
            if verbose {
                eprintln!(
                    "watch_all: desktop recent targets={} names={}",
                    targets.len(),
                    targets
                        .iter()
                        .map(|target| target.name.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                );
            }
            targets
        }
        Err(err) => {
            if verbose {
                eprintln!("watch_all: desktop recent targets failed error={err:#}");
            }
            Vec::new()
        }
    }
}

fn desktop_recent_chat_limit(queue_targets: Option<&BTreeSet<String>>) -> usize {
    queue_targets
        .map(|targets| targets.len().max(DESKTOP_RECENT_CHAT_LIMIT))
        .unwrap_or(DESKTOP_RECENT_CHAT_LIMIT)
}

fn filter_chats_by_queue(
    chats: Vec<ChatSummary>,
    queue_targets: Option<&BTreeSet<String>>,
) -> Vec<ChatSummary> {
    let Some(queue_targets) = queue_targets else {
        return chats;
    };

    chats
        .into_iter()
        .filter(|chat| {
            queue_targets.contains(&chat.chat_id) || queue_targets.contains(&chat.chat_name)
        })
        .collect()
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

fn desktop_preview_to_value(text: &str) -> Value {
    json!({
        "userid": "desktop",
        "msgtype": "text",
        "text": {
            "content": text,
        },
        "desktop": {
            "key": desktop_preview_key(text),
            "source": "macos_accessibility_preview",
        },
    })
}

fn desktop_message_was_emitted_from_preview(
    chat_id: &str,
    message: &Value,
    state: &WatchState,
) -> bool {
    message
        .pointer("/text/content")
        .and_then(Value::as_str)
        .is_some_and(|text| {
            state
                .seen
                .contains(&desktop_preview_dedup_key(chat_id, text))
        })
}

fn desktop_chat_id(target: &str) -> String {
    format!("desktop:{target}")
}

fn desktop_preview_key(text: &str) -> String {
    format!("preview|{text}")
}

fn desktop_preview_dedup_key(chat_id: &str, text: &str) -> String {
    format!("{chat_id}|desktop|{}", desktop_preview_key(text))
}

fn desktop_message_texts(messages: &[DesktopChatMessage]) -> Vec<String> {
    messages
        .iter()
        .map(|message| message.text.clone())
        .collect()
}

fn desktop_new_message_start(
    previous: Option<&Vec<String>>,
    current: &[String],
    initial_preview: Option<&str>,
) -> usize {
    if current.is_empty() {
        return 0;
    }

    if let Some(previous) = previous {
        if previous.is_empty() {
            return current.len().saturating_sub(1);
        }

        let max_overlap = previous.len().min(current.len());
        for overlap in (1..=max_overlap).rev() {
            if previous[previous.len() - overlap..] == current[..overlap] {
                return overlap;
            }
        }
    }

    if let Some(initial_preview) = initial_preview {
        if let Some(index) = current.iter().rposition(|text| text == initial_preview) {
            return index + 1;
        }
    }

    current.len().saturating_sub(1)
}

fn should_force_desktop_sweep(poll_index: u64, interval_sec: u64) -> bool {
    if poll_index == 0 {
        return false;
    }

    poll_index % desktop_full_sweep_interval(interval_sec) == 0
}

fn desktop_full_sweep_interval(interval_sec: u64) -> u64 {
    let interval_sec = interval_sec.max(1);
    DESKTOP_FULL_SWEEP_SECONDS.div_ceil(interval_sec).max(1)
}

fn watch_all_poll_window(interval_sec: u64) -> (String, String) {
    recent_seconds_window(watch_all_window_seconds(interval_sec))
}

fn watch_all_now_time() -> String {
    recent_seconds_window(0).1
}

fn watch_all_window_seconds(interval_sec: u64) -> u64 {
    interval_sec.saturating_mul(3).max(60)
}

fn chat_needs_fetch(
    chat: &ChatSummary,
    chat_signatures: &BTreeMap<String, ChatPollSignature>,
) -> bool {
    let Some(signature) = ChatPollSignature::from_chat(chat) else {
        return true;
    };
    chat_signatures.get(&chat.chat_id) != Some(&signature)
}

impl ChatPollSignature {
    fn from_chat(chat: &ChatSummary) -> Option<Self> {
        if chat.last_msg_time.is_none() && chat.msg_count.is_none() {
            return None;
        }

        Some(Self {
            last_msg_time: chat.last_msg_time.clone(),
            msg_count: chat.msg_count,
        })
    }
}

fn update_chat_signature(
    chat_signatures: &mut BTreeMap<String, ChatPollSignature>,
    chat: &ChatSummary,
) {
    match ChatPollSignature::from_chat(chat) {
        Some(signature) => {
            chat_signatures.insert(chat.chat_id.clone(), signature);
        }
        None => {
            chat_signatures.remove(&chat.chat_id);
        }
    }
}

fn watch_target_meta_from_chat(chat: &ChatSummary) -> WatchTargetMeta {
    let target = if chat.chat_name.is_empty() {
        chat.chat_id.clone()
    } else {
        chat.chat_name.clone()
    };

    WatchTargetMeta {
        target,
        chat_id: chat.chat_id.clone(),
        chat_name: chat.chat_name.clone(),
    }
}

fn take_unseen_message_key(
    state: &mut WatchState,
    chat_id: &str,
    message: &Value,
    baseline_chat: bool,
    pending_output_keys: &mut BTreeSet<String>,
) -> Option<String> {
    let key = message_dedup_key(chat_id, message);
    if state.seen.contains(&key) || pending_output_keys.contains(&key) {
        return None;
    }

    if baseline_chat {
        state.seen.insert(key);
        return None;
    }

    pending_output_keys.insert(key.clone());
    Some(key)
}

async fn wait_next_poll_or_idle(
    options: &WatchAllOptions,
    last_emit_at: Instant,
    poll_deadline: Instant,
) -> bool {
    if options.idle_timeout_sec == 0 {
        tokio::time::sleep_until(poll_deadline).await;
        return false;
    }

    let idle_deadline = last_emit_at + StdDuration::from_secs(options.idle_timeout_sec);
    tokio::select! {
        _ = tokio::time::sleep_until(poll_deadline) => false,
        _ = tokio::time::sleep_until(idle_deadline) => {
            if options.verbose {
                eprintln!(
                    "watch_all: idle timeout reached after {}s without new messages",
                    options.idle_timeout_sec
                );
            }
            true
        }
    }
}

fn save_state_or_log(state_file: &Path, state: &WatchState) {
    if let Err(err) = save_state(state_file, state) {
        eprintln!(
            "watch_all: save state failed state_file=\"{}\" error={:#}",
            state_file.display(),
            err
        );
    }
}

fn watch_all_state_file(save_dir: &Path) -> PathBuf {
    save_dir.join(".watch_all.json")
}

fn watch_all_queue_file(save_dir: &Path) -> PathBuf {
    save_dir.join(".watch_all_queue.txt")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_chat(
        id: &str,
        name: &str,
        last_msg_time: Option<&str>,
        msg_count: Option<u64>,
    ) -> ChatSummary {
        ChatSummary {
            chat_id: id.to_string(),
            chat_name: name.to_string(),
            last_msg_time: last_msg_time.map(ToString::to_string),
            msg_count,
        }
    }

    #[test]
    fn watch_all_args_uses_ten_second_default_interval() {
        let matches = WatchAllArgs::augment_args(Command::new("test"))
            .try_get_matches_from(["test"])
            .unwrap();
        let args = WatchAllArgs::from_arg_matches(&matches).unwrap();

        assert_eq!(args.interval_sec, 10);
    }

    #[test]
    fn watch_all_args_rejects_zero_interval() {
        let matches = WatchAllArgs::augment_args(Command::new("test"))
            .try_get_matches_from(["test", "--interval-sec", "0"])
            .unwrap();
        let args = WatchAllArgs::from_arg_matches(&matches).unwrap();

        let err = validate_watch_all_args(&args).unwrap_err().to_string();
        assert!(err.contains("--interval-sec"));
    }

    #[test]
    fn watch_all_args_accepts_queue_targets() {
        let matches = WatchAllArgs::augment_args(Command::new("test"))
            .try_get_matches_from(["test", "--to", "邹友", "--to", "友友"])
            .unwrap();
        let args = WatchAllArgs::from_arg_matches(&matches).unwrap();

        assert_eq!(args.to, vec!["邹友".to_string(), "友友".to_string()]);
    }

    #[test]
    fn parse_watch_queue_content_accepts_lines_and_comments() {
        let targets = parse_watch_queue_content("邹友\n# 注释\n友友 # 行内注释\n\n").unwrap();

        assert_eq!(
            targets,
            BTreeSet::from(["友友".to_string(), "邹友".to_string()])
        );
    }

    #[test]
    fn parse_watch_queue_content_accepts_json_array() {
        let targets = parse_watch_queue_content(r#"["邹友", "友友", "邹友"]"#).unwrap();

        assert_eq!(
            targets,
            BTreeSet::from(["友友".to_string(), "邹友".to_string()])
        );
    }

    #[test]
    fn watch_all_window_uses_minimum_overlap() {
        assert_eq!(watch_all_window_seconds(10), 60);
        assert_eq!(watch_all_window_seconds(30), 90);
    }

    #[test]
    fn desktop_new_message_start_uses_sequence_overlap() {
        let previous = vec!["1".to_string(), "2".to_string(), "3".to_string()];
        let current = vec!["2".to_string(), "3".to_string(), "3".to_string()];

        assert_eq!(
            desktop_new_message_start(Some(&previous), &current, None),
            2
        );
    }

    #[test]
    fn desktop_new_message_start_without_overlap_keeps_latest_only() {
        let previous = vec!["1".to_string(), "2".to_string()];
        let current = vec!["3".to_string(), "4".to_string()];

        assert_eq!(
            desktop_new_message_start(Some(&previous), &current, None),
            1
        );
        assert_eq!(desktop_new_message_start(None, &current, None), 1);
    }

    #[test]
    fn desktop_new_message_start_uses_initial_preview_before_snapshot() {
        let current = vec![
            "旧消息".to_string(),
            "启动时预览".to_string(),
            "你在监控吗".to_string(),
            "你好".to_string(),
        ];

        assert_eq!(
            desktop_new_message_start(None, &current, Some("启动时预览")),
            2
        );
    }

    #[test]
    fn desktop_full_sweep_runs_about_once_per_minute() {
        assert_eq!(desktop_full_sweep_interval(5), 12);
        assert_eq!(desktop_full_sweep_interval(60), 1);
        assert!(!should_force_desktop_sweep(0, 5));
        assert!(!should_force_desktop_sweep(11, 5));
        assert!(should_force_desktop_sweep(12, 5));
    }

    #[test]
    fn desktop_recent_limit_scales_with_queue_size() {
        let targets = (0..16).map(|index| format!("用户{index}")).collect();

        assert_eq!(desktop_recent_chat_limit(Some(&targets)), 16);
        assert_eq!(desktop_recent_chat_limit(None), DESKTOP_RECENT_CHAT_LIMIT);
    }

    #[test]
    fn select_missing_queue_targets_rotates_from_cursor() {
        let queue = BTreeSet::from(["ZGBin".to_string(), "友友".to_string(), "邹友".to_string()]);
        let visible = BTreeSet::from(["友友".to_string()]);

        let missing = select_missing_queue_targets(Some(&queue), &visible, 1);

        assert_eq!(missing, vec!["邹友".to_string(), "ZGBin".to_string()]);
    }

    #[test]
    fn filter_chats_by_queue_matches_id_or_name() {
        let chats = vec![
            make_chat("chat1", "张三", None, None),
            make_chat("chat2", "李四", None, None),
        ];
        let queue = BTreeSet::from(["chat1".to_string(), "李四".to_string()]);

        let filtered = filter_chats_by_queue(chats, Some(&queue));

        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn baseline_marks_existing_messages_without_emitting() {
        let mut state = WatchState::default();
        let mut pending_output_keys = BTreeSet::new();
        let message = json!({
            "userid": "lisi",
            "send_time": "2026-03-17 09:40:00",
            "msgtype": "text",
            "text": {
                "content": "历史消息"
            }
        });

        let key = take_unseen_message_key(
            &mut state,
            "chat1",
            &message,
            true,
            &mut pending_output_keys,
        );

        assert!(key.is_none());
        assert!(
            state
                .seen
                .contains("chat1|2026-03-17 09:40:00|lisi|text|历史消息")
        );
        assert!(pending_output_keys.is_empty());
    }

    #[test]
    fn new_message_emits_once_after_state_ack() {
        let mut state = WatchState::default();
        let mut pending_output_keys = BTreeSet::new();
        let message = json!({
            "userid": "lisi",
            "send_time": "2026-03-17 09:41:00",
            "msgtype": "text",
            "text": {
                "content": "新消息"
            }
        });

        let key = take_unseen_message_key(
            &mut state,
            "chat1",
            &message,
            false,
            &mut pending_output_keys,
        )
        .unwrap();
        assert_eq!(key, "chat1|2026-03-17 09:41:00|lisi|text|新消息");
        assert!(
            take_unseen_message_key(
                &mut state,
                "chat1",
                &message,
                false,
                &mut pending_output_keys,
            )
            .is_none()
        );

        state.seen.insert(key);
        pending_output_keys.clear();
        assert!(
            take_unseen_message_key(
                &mut state,
                "chat1",
                &message,
                false,
                &mut pending_output_keys,
            )
            .is_none()
        );
    }

    #[test]
    fn chat_signature_skips_unchanged_chat_when_available() {
        let chat = make_chat("chat1", "张三", Some("2026-03-17 09:41:00"), Some(2));
        let mut signatures = BTreeMap::new();

        assert!(chat_needs_fetch(&chat, &signatures));
        update_chat_signature(&mut signatures, &chat);
        assert!(!chat_needs_fetch(&chat, &signatures));

        let changed = make_chat("chat1", "张三", Some("2026-03-17 09:42:00"), Some(3));
        assert!(chat_needs_fetch(&changed, &signatures));
    }

    #[test]
    fn chat_without_signature_fields_is_always_fetched() {
        let chat = make_chat("chat1", "张三", None, None);
        let signatures = BTreeMap::new();

        assert!(chat_needs_fetch(&chat, &signatures));
    }

    #[test]
    fn watch_all_target_uses_chat_name() {
        let meta = watch_target_meta_from_chat(&make_chat("chat1", "张三", None, None));

        assert_eq!(meta.target, "张三");
        assert_eq!(meta.chat_id, "chat1");
        assert_eq!(meta.chat_name, "张三");
    }

    #[test]
    fn watch_all_state_file_defaults_under_save_dir() {
        let dir = Path::new("/tmp/wecom/media");

        assert_eq!(watch_all_state_file(dir), dir.join(".watch_all.json"));
    }
}
