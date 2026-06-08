use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use clap::ArgMatches;

use crate::helpers::contact::add_external_friend::AddExternalFriendHelper;
use crate::helpers::doc::smartpage_create::SmartpageCreateHelper;
use crate::helpers::doc::smartsheet_add_records_auto_file::SmartsheetAddRecordsAutoFileHelper;
use crate::helpers::doc::smartsheet_update_records_auto_file::SmartsheetUpdateRecordsAutoFileHelper;
use crate::helpers::msg::send_friend_message::SendFriendMessageHelper;
use crate::helpers::msg::watch_all::WatchAllHelper;
use crate::helpers::msg::watch_friend::WatchFriendHelper;

/// Helper trait：每个 helper 需要实现此 trait。
/// `execute` 返回 boxed future 以保证 dyn 兼容（object safety）。
pub trait Helper: Send + Sync {
    fn category(&self) -> &'static str;

    fn command(&self) -> clap::Command;

    fn execute<'a>(
        &'a self,
        matches: &'a ArgMatches,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
}

pub struct HelperRegistry {
    helpers: Vec<Box<dyn Helper>>,
}

impl HelperRegistry {
    pub fn new() -> Self {
        let helpers: Vec<Box<dyn Helper>> = vec![
            Box::new(AddExternalFriendHelper),
            Box::new(SmartpageCreateHelper),
            Box::new(SmartsheetAddRecordsAutoFileHelper),
            Box::new(SmartsheetUpdateRecordsAutoFileHelper),
            Box::new(SendFriendMessageHelper),
            Box::new(WatchAllHelper),
            Box::new(WatchFriendHelper),
        ];
        Self { helpers }
    }

    pub fn get(&self, category: &str, name: &str) -> Option<&dyn Helper> {
        self.helpers
            .iter()
            .find(|h| h.category() == category && h.command().get_name() == name)
            .map(|h| &**h)
    }

    pub fn list_in_category(&self, category: &str) -> Vec<&dyn Helper> {
        self.helpers
            .iter()
            .filter(|h| h.category() == category)
            .map(|h| &**h)
            .collect()
    }
}
