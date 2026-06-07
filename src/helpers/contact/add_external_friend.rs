use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use clap::{ArgMatches, Args, Command, FromArgMatches};

use crate::helpers::registry::Helper;
use crate::helpers::wecom_desktop::{AddExternalFriendRequest, default_driver, validate_phone};

#[derive(Args, Debug)]
pub struct AddExternalFriendArgs {
    /// 外部联系人手机号，可包含 + 国际区号、空格或短横线
    #[arg(long)]
    pub phone: String,

    /// 添加后设置的备注名
    #[arg(long)]
    pub remark: Option<String>,

    /// 发送给对方的验证语
    #[arg(long)]
    pub greeting: Option<String>,
}

pub struct AddExternalFriendHelper;

impl Helper for AddExternalFriendHelper {
    fn category(&self) -> &'static str {
        "contact"
    }

    fn command(&self) -> clap::Command {
        AddExternalFriendArgs::augment_args(
            Command::new("+add_external_friend")
                .about("通过 macOS 企业微信客户端按手机号添加外部联系人"),
        )
    }

    fn execute<'a>(
        &'a self,
        matches: &'a ArgMatches,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async {
            let args = AddExternalFriendArgs::from_arg_matches(matches)?;
            let request = AddExternalFriendRequest {
                phone: validate_phone(&args.phone)?,
                remark: normalize_optional_text(args.remark),
                greeting: normalize_optional_text(args.greeting),
            };

            let result = default_driver().add_external_friend(&request)?;
            println!("{}", serde_json::to_string(&result)?);

            Ok(())
        })
    }
}

fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}
