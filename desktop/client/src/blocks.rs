//! Blocking people (#565, web/src/blocks/queries.ts). A block or unblock on
//! any of the user's devices arrives as the account frame `bl`, which reads
//! the list again; the DM list follows, since a DM says whether it is
//! blocked.

use uuid::Uuid;

use crate::Client;
use crate::store::{Notice, OpState};

impl Client {
    /// `GET /api/users/me/blocks`. Op key `blocks`.
    pub fn load_blocks(&self) {
        let http = self.http().clone();
        self.run("blocks", async move { http.blocks().await }, |s, list| {
            s.blocks = Some(list)
        });
    }

    /// The `bl` frame, and the end of a block or unblock here: the blocks,
    /// the DMs (their `blocked`/`closed`), friends and message requests,
    /// which a block may have cut.
    pub(crate) fn blocks_changed(&self) {
        self.load_blocks();
        let http = self.http().clone();
        self.run("dms_blocks", async move { http.dms().await }, |s, list| {
            s.dms = list
        });
        self.load_friends();
        self.load_dm_requests();
    }

    /// Blocks `user`. Op key `block:{user}`; the lists are read again
    /// either way.
    pub fn block_user(&self, user: Uuid) {
        let this = self.clone();
        self.spawn(async move {
            let http = this.http().clone();
            let key = format!("block:{user}");
            if this.op(&key, http.block_user(user)).await.is_none() {
                this.failed(&key, gelabber_i18n::blocks::block_failed());
            }
            this.blocks_changed();
        });
    }

    /// Unblocks `user`. Op key `unblock:{user}`.
    pub fn unblock_user(&self, user: Uuid) {
        let this = self.clone();
        self.spawn(async move {
            let http = this.http().clone();
            let key = format!("unblock:{user}");
            if this.op(&key, http.unblock_user(user)).await.is_none() {
                this.failed(&key, gelabber_i18n::blocks::unblock_failed());
            }
            this.blocks_changed();
        });
    }

    /// The toast for a block or unblock the server turned down.
    fn failed(&self, key: &str, failed: &'static str) {
        if let Some(OpState::Failed(error)) = self.with_state(|s| s.op(key).cloned()) {
            self.notify(Notice::Failed {
                failed,
                gone: None,
                error,
            });
        }
    }
}
