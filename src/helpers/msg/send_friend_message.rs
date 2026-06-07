use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use anyhow::Result;
use clap::{ArgGroup, ArgMatches, Args, Command, FromArgMatches};

use crate::helpers::registry::Helper;
use crate::helpers::wecom_desktop::{
    FileUsage, FriendMessagePayload, SendFriendMessageRequest, default_driver,
    validate_existing_file, validate_target, validate_text_message,
};

#[derive(Args, Debug)]
#[command(group(
    ArgGroup::new("payload")
        .required(true)
        .multiple(false)
        .args(["text", "image", "file"])
))]
pub struct SendFriendMessageArgs {
    /// 好友名称或备注名
    #[arg(long)]
    pub to: String,

    /// 要发送的文本消息
    #[arg(long)]
    pub text: Option<String>,

    /// 要发送的图片路径
    #[arg(long)]
    pub image: Option<PathBuf>,

    /// 要发送的文件路径
    #[arg(long)]
    pub file: Option<PathBuf>,
}

pub struct SendFriendMessageHelper;

impl Helper for SendFriendMessageHelper {
    fn category(&self) -> &'static str {
        "msg"
    }

    fn command(&self) -> clap::Command {
        SendFriendMessageArgs::augment_args(
            Command::new("+send_friend_message")
                .about("通过 macOS 企业微信客户端给指定好友发送文本、图片或文件"),
        )
    }

    fn execute<'a>(
        &'a self,
        matches: &'a ArgMatches,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async {
            let args = SendFriendMessageArgs::from_arg_matches(matches)?;
            let payload = if let Some(text) = args.text.as_deref() {
                FriendMessagePayload::Text(validate_text_message(text)?)
            } else if let Some(path) = args.image.as_deref() {
                FriendMessagePayload::Image(validate_existing_file(path, FileUsage::Image)?)
            } else if let Some(path) = args.file.as_deref() {
                FriendMessagePayload::File(validate_existing_file(path, FileUsage::File)?)
            } else {
                unreachable!("clap ArgGroup requires one payload")
            };

            let request = SendFriendMessageRequest {
                to: validate_target(&args.to)?,
                payload,
            };

            let result = default_driver().send_friend_message(&request)?;
            println!("{}", serde_json::to_string(&result)?);

            Ok(())
        })
    }
}
