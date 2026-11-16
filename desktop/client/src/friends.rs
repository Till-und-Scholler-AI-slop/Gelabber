//! Friends, friend codes and message requests (#563, web/src/friends/
//! queries.ts). Every change on any device arrives as an account frame:
//! `fr` reloads the friend list, `mr` the incoming message requests.

use uuid::Uuid;

use crate::Client;
use crate::api::FriendTarget;
use crate::models::FriendAnswer;
use crate::store::Notice;

/// A friend action on someone, with its op key `friend_<kind>:{user}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FriendAction {
    Accept,
    Decline,
    Withdraw,
    Remove,
}

impl FriendAction {
    fn key(self) -> &'static str {
        match self {
            Self::Accept => "friend_accept",
            Self::Decline => "friend_decline",
            Self::Withdraw => "friend_withdraw",
            Self::Remove => "friend_remove",
        }
    }

    /// The op key of this action on `user`, for a button's pending state.
    pub fn op(self, user: Uuid) -> String {
        format!("{}:{user}", self.key())
    }
}

impl Client {
    /// `GET /api/friends`. Op key `friends`.
    pub fn load_friends(&self) {
        let http = self.http().clone();
        self.run("friends", async move { http.friends().await }, |s, list| {
            s.friends = Some(list)
        });
    }

    /// `GET /api/dms/requests`. Op key `dm_requests`.
    pub fn load_dm_requests(&self) {
        let http = self.http().clone();
        self.run(
            "dm_requests",
            async move { http.dm_requests().await },
            |s, list| s.dm_requests = Some(list),
        );
    }

    /// `GET /api/me/friend-code`. Op key `friend_code`.
    pub fn load_friend_code(&self) {
        let http = self.http().clone();
        self.run(
            "friend_code",
            async move { http.friend_code().await },
            |s, code| s.friend_code = Some(code.code),
        );
    }

    /// `POST /api/me/friend-code`. Op key `friend_code_new`.
    pub fn regenerate_friend_code(&self) {
        let http = self.http().clone();
        self.run(
            "friend_code_new",
            async move { http.regenerate_friend_code().await },
            |s, code| s.friend_code = Some(code.code),
        );
    }

    /// Sends a friend request by profile or code. Op key `friend_send`;
    /// the toast names the person, or says they are friends now when they
    /// had asked already.
    pub fn send_friend_request(&self, target: FriendTarget) {
        let this = self.clone();
        self.spawn(async move {
            let http = this.http().clone();
            let answer: Option<FriendAnswer> = this
                .op("friend_send", http.send_friend_request(target))
                .await;
            if let Some(answer) = answer {
                this.notify(Notice::Info(if answer.status == "friends" {
                    gelabber_i18n::friends::add_friends(&answer.peer.name)
                } else {
                    gelabber_i18n::friends::add_sent(&answer.peer.name)
                }));
            }
            this.load_friends();
        });
    }

    /// Accepts, declines, withdraws or ends a friendship with `user`; the
    /// list is read again either way, as the web does.
    pub fn friend_action(&self, action: FriendAction, user: Uuid) {
        let this = self.clone();
        self.spawn(async move {
            let http = this.http().clone();
            let key = action.op(user);
            match action {
                FriendAction::Accept => this.op(&key, http.accept_friend(user)).await,
                FriendAction::Decline => this.op(&key, http.decline_friend(user)).await,
                FriendAction::Withdraw => this.op(&key, http.withdraw_friend(user)).await,
                FriendAction::Remove => this.op(&key, http.remove_friend(user)).await,
            };
            this.load_friends();
        });
    }

    /// Accepts a message request: it becomes an ordinary DM in the list.
    /// Op key `dm_accept:{channel}`.
    pub fn accept_dm_request(&self, channel: Uuid) {
        let http = self.http().clone();
        self.run(
            format!("dm_accept:{channel}"),
            async move { http.accept_dm_request(channel).await },
            move |s, dm| {
                if let Some(list) = &mut s.dm_requests {
                    list.retain(|d| d.id != channel);
                }
                match s.dms.iter_mut().find(|d| d.id == dm.id) {
                    Some(known) => *known = dm,
                    None => s.dms.insert(0, dm),
                }
            },
        );
    }

    /// Declines quietly: the request leaves the list; the sender is not
    /// told. Op key `dm_decline:{channel}`.
    pub fn decline_dm_request(&self, channel: Uuid) {
        let http = self.http().clone();
        self.run(
            format!("dm_decline:{channel}"),
            async move { http.decline_dm_request(channel).await },
            move |s, ()| {
                if let Some(list) = &mut s.dm_requests {
                    list.retain(|d| d.id != channel);
                }
                s.dms.retain(|d| d.id != channel);
            },
        );
    }
}
