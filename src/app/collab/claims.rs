//! The collaboration panel's app-side glue: the `PIDCOLLAB` / `PIDCLAIM` /
//! `PIDRELEASE` commands, the panel's own messages, and the two synchronisations
//! the panel needs — the cached view it draws, and the scene colouring of tags
//! another session holds.
//!
//! The panel is a projection of [`crate::app::collab::State`]: it holds no
//! session state of its own, so a server push, a reconnect or a release needs no
//! extra bookkeeping beyond refreshing the cached view.

use crate::app::{Message, OpenCADStudio};
use crate::ui::dock::PanelId;
use crate::ui::window::pid_collab::{ClaimRow, CollabView, ConflictRow, PeerRow, PidCollabMsg};
use iced::Task;

impl OpenCADStudio {
    /// Rebuild the cached panel view from the live session state.
    ///
    /// Called after every collaboration event and on every panel interaction,
    /// so the panel never draws a stale roster or a released claim.
    pub(crate) fn refresh_collab_view(&mut self) {
        let state = &self.collab;
        // This session first, then the others. The roster the server sends
        // is the others (`welcome.peers` does not list the session it
        // welcomes), so the row for this session is the panel's own.
        let own = state.session_id.as_ref().map(|_| PeerRow {
            user_id: state.user_id.clone(),
            display_name: state.user_id.clone(),
            is_self: true,
            // The marker is drawn for a session that lost the edit right,
            // on the row of whoever kept it; that is never its own row.
            is_holder: false,
        });
        let others = state.peers.iter().map(|peer| PeerRow {
            user_id: peer.user_id.clone(),
            display_name: peer.display_name.clone(),
            is_self: false,
            // The server names the holding session, so the same user's
            // other session is not marked along with it.
            is_holder: state.readonly
                && state
                    .readonly_holder
                    .as_deref()
                    .is_some_and(|holder| holder == peer.session_id),
        });
        let peers: Vec<PeerRow> = own.into_iter().chain(others).collect();
        let claims: Vec<ClaimRow> = state
            .claims
            .iter()
            .map(|(tag, holder)| ClaimRow {
                tag: tag.clone(),
                holder: holder.clone(),
                is_self: *holder == state.user_id,
            })
            .collect();
        // A row that is no longer drawn must stop reporting its old rectangle,
        // or a bounds reader sees a claim the panel has already dropped. The
        // claim rows are rebuilt below on the next frame, so forgetting the
        // whole prefix here is exact rather than lossy.
        if self.pid_collab_view.claims != claims {
            crate::ui::wrap_bar::forget_bounds(
                crate::ui::window::pid_collab::ROW_BOUNDS_PREFIX,
            );
        }
        // Likewise the read-only badge and the edit-right marker: a session
        // that reconnects is no longer downgraded, and a reader of the bounds
        // must not find a badge the panel stopped drawing.
        if self.pid_collab_view.readonly != state.readonly || self.pid_collab_view.peers != peers {
            crate::ui::wrap_bar::forget_bounds(crate::ui::window::pid_collab::READONLY_BOUNDS_ID);
            crate::ui::wrap_bar::forget_bounds(crate::ui::window::pid_collab::HOLDER_BOUNDS_PREFIX);
        }
        // The badge is worn while the drawing is not the room's: parted
        // from it, or not taken on the connection that is up.
        let mismatch = state.mismatch();
        if self.pid_collab_view.diverged.is_some() != mismatch.is_some() {
            crate::ui::wrap_bar::forget_bounds(crate::ui::window::pid_collab::DIVERGED_BOUNDS_ID);
        }
        let conflicts: Vec<ConflictRow> = state
            .conflicts
            .iter()
            .map(|entry| ConflictRow {
                conflict_id: entry.conflict_id.clone(),
                object_ref: entry.object_ref.clone(),
                field: entry.field.clone(),
                value_a: entry.value_a.clone(),
                value_b: entry.value_b.clone(),
            })
            .collect();
        // Nor does a panel that has a server go on reporting the note it
        // drew when it had none.
        if self.pid_collab_view.configured() != state.wanted() {
            crate::ui::wrap_bar::forget_bounds(
                crate::ui::window::pid_collab::NOT_CONFIGURED_BOUNDS_ID,
            );
        }
        // An entry that has been ruled on stops reporting its rectangle,
        // for the same reason a released claim does.
        if self.pid_collab_view.conflicts != conflicts {
            crate::ui::wrap_bar::forget_bounds(
                crate::ui::window::pid_collab::CONFLICT_BOUNDS_PREFIX,
            );
        }
        // The line under the banner, likewise, once nothing is left unsent.
        let sync = self.collab_sync_line();
        if self.pid_collab_view.sync.is_some() != sync.is_some() {
            crate::ui::wrap_bar::forget_bounds(crate::ui::window::pid_collab::SYNC_BOUNDS_ID);
        }
        self.pid_collab_view = CollabView {
            connected: state.connected,
            server_url: state.server_url.clone(),
            session_id: state.session_id.clone(),
            readonly: state.readonly,
            diverged: mismatch,
            sync,
            peers,
            conflicts,
            claims,
            last_message: state.last_claim_message.clone(),
            claims_open: self.pid_collab_claims_open,
        };
    }

    /// Colour the entities whose tag a session holds, so the drawing and the
    /// panel agree about what is occupied.
    ///
    /// Every held tag is coloured, including this session's own: the colour
    /// marks the entity as *occupied* — what any peer sees — and
    /// `VAL-CLI-008` reads the tint of the symbol `u1` itself claimed. The
    /// tag→handles resolution reads each entity's `PID_AUTHOR` record, the same
    /// source the tag engine and the properties panel use.
    ///
    /// Only the session's drawing is coloured. A claim is on a tag of that
    /// drawing; another sheet open beside it may carry the same tag and is
    /// not occupied by anyone.
    pub(crate) fn sync_claim_colors(&mut self) {
        let tags: Vec<String> = self.collab.claims.keys().cloned().collect();
        let session_tab = if tags.is_empty() {
            None
        } else {
            self.collab_tab()
        };
        let wanted: std::collections::HashSet<&str> = tags.iter().map(String::as_str).collect();
        for (index, tab) in self.tabs.iter_mut().enumerate() {
            if session_tab != Some(index) {
                tab.scene.set_claim_display(None);
                continue;
            }
            let document = &tab.scene.document;
            let handles: rustc_hash::FxHashSet<document::Handle> = document
                .entities()
                .filter_map(|entity| {
                    let handle = entity.common().handle;
                    let author = crate::pid::persist::PidAuthor::load(document, handle)?;
                    wanted.contains(author.tag.as_str()).then_some(handle)
                })
                .collect();
            let display = (!handles.is_empty()).then_some(crate::scene::ClaimDisplay {
                handles,
                color: crate::scene::CLAIM_COLOR,
            });
            tab.scene.set_claim_display(display);
        }
    }

    /// Show the collaboration panel where the layout has it, docking it on the
    /// right the first time. `PIDCOLLAB` uses this to show; the panel's own
    /// close button hides it.
    pub(in crate::app) fn show_pid_collab_panel(&mut self) -> Task<Message> {
        if self.dock.location(PanelId::PidCollab).is_none() {
            self.dock
                .dock(PanelId::PidCollab, crate::app::config::DockSide::Right, 0);
        }
        self.show_pid_collab = true;
        self.refresh_collab_view();
        self.dock_reveal(PanelId::PidCollab);
        Task::none()
    }

    /// `PIDCOLLAB` — the panel, and the three settings a session is made of.
    ///
    /// * bare or `ON` shows the panel, `OFF` hides it, so the command has a
    ///   deterministic form for scripted runs;
    /// * `SERVER <ws://host:port/ws>` names the collaboration server, and
    ///   `SERVER OFF` forgets it;
    /// * `USER <id>` says who this is;
    /// * `DRAWING <id>` says which drawing the session is for, where the
    ///   file name does not.
    ///
    /// The three are saved as they are given — to the settings file the next
    /// start reads — and a run that has no session yet starts one as soon as
    /// it has all three, on the drawing in front (`VAL-CROSS-006`). A run
    /// that has its session keeps it: what is given then is for the next
    /// start.
    pub(in crate::app) fn dispatch_pidcollab(&mut self, cmd: &str, _i: usize) -> Option<Task<Message>> {
        let verb_end = cmd
            .trim_start()
            .find(char::is_whitespace)
            .unwrap_or(cmd.trim_start().len());
        let verb = &cmd.trim_start()[..verb_end];
        if !verb.eq_ignore_ascii_case("PIDCOLLAB") {
            return None;
        }
        let arg = cmd.trim_start()[verb_end..].trim();
        let (word, value) = match arg.split_once(char::is_whitespace) {
            Some((word, value)) => (word, value.trim()),
            None => (arg, ""),
        };
        match (word.to_ascii_uppercase().as_str(), value) {
            ("" | "ON", "") => Some(self.show_pid_collab_panel()),
            ("OFF", "") => Some(self.on_dock(crate::ui::dock::DockMsg::Close(PanelId::PidCollab))),
            ("SERVER", value) => {
                self.set_collab_setting(CollabSetting::Server, value);
                Some(Task::none())
            }
            ("USER", value) => {
                self.set_collab_setting(CollabSetting::User, value);
                Some(Task::none())
            }
            ("DRAWING", value) => {
                self.set_collab_setting(CollabSetting::Drawing, value);
                Some(Task::none())
            }
            ("UNSYNCED", "") => {
                self.list_unsynced();
                Some(Task::none())
            }
            ("UNSYNCED", value) if value.eq_ignore_ascii_case("CLEAR") => {
                self.clear_unsynced();
                Some(Task::none())
            }
            ("EXPORT", value) => {
                self.export_unsynced(value);
                Some(Task::none())
            }
            ("DETACH", value) if value.is_empty() || value.eq_ignore_ascii_case("FORCE") => {
                self.detach(!value.is_empty());
                Some(Task::none())
            }
            _ => {
                // The word only: what follows it may be an address, and an
                // address is not echoed as typed (`shown_address`).
                self.command_line.push_error(&format!(
                    "PIDCOLLAB: 只接受 ON、OFF、SERVER <ws://主机:端口/ws>、USER <用户>、\
                     DRAWING <图号>、UNSYNCED、UNSYNCED CLEAR、EXPORT <路径>、\
                     DETACH [FORCE]，实得 \"{word}\"。"
                ));
                Some(Task::none())
            }
        }
    }

    /// Take one collaboration setting as typed: check it, keep it in what
    /// the settings file is written from, write the file, and — when this
    /// run has no session yet — start the session if nothing is missing now.
    fn set_collab_setting(&mut self, setting: CollabSetting, typed: &str) {
        let typed = typed.trim();
        let name = setting.name();
        if typed.is_empty() {
            let saved = match setting {
                // Not as a file edited by hand may have it: see `shown_address`.
                CollabSetting::Server => super::shown_address(&self.collab.configured_server_url),
                CollabSetting::User => self.collab.configured_user_id.clone(),
                CollabSetting::Drawing => self.collab.configured_drawing_id.clone(),
            };
            let saved = if saved.is_empty() { "（未设置）" } else { saved.as_str() };
            self.command_line
                .push_info(&format!("PIDCOLLAB {name}: 已保存的是 {saved}"));
            return;
        }
        let value = match setting {
            CollabSetting::Server if typed.eq_ignore_ascii_case("OFF") => String::new(),
            CollabSetting::Server => match dialable(typed) {
                Ok(()) => typed.to_owned(),
                Err(why) => {
                    self.command_line.push_error(&format!(
                        "PIDCOLLAB SERVER: {why}。地址要写成 ws://主机:端口/ws，实得 \"{}\"。",
                        super::shown_address(typed)
                    ));
                    return;
                }
            },
            CollabSetting::User | CollabSetting::Drawing => {
                if typed.contains(char::is_whitespace) {
                    self.command_line.push_error(&format!(
                        "PIDCOLLAB {name}: 只接受一个词，实得 \"{typed}\"。"
                    ));
                    return;
                }
                typed.to_owned()
            }
        };

        // A run without a session takes the value for itself as well; one
        // with a session leaves the session what it is.
        let in_session = self.collab.wanted();
        match setting {
            CollabSetting::Server => self.collab.configured_server_url = value.clone(),
            CollabSetting::User => {
                self.collab.configured_user_id = value.clone();
                if !in_session {
                    self.collab.user_id = value.clone();
                }
            }
            CollabSetting::Drawing => {
                self.collab.configured_drawing_id = value.clone();
                if !in_session {
                    self.collab.drawing_id = value.clone();
                    self.collab.drawing_given = !value.is_empty();
                }
            }
        }
        self.save_config();
        let shown = if value.is_empty() { "（无）" } else { value.as_str() };
        if in_session {
            self.command_line.push_info(&format!(
                "协同：{name} 已保存为 {shown}；本次运行的会话不变，下次启动生效"
            ));
            return;
        }
        match self.start_saved_session() {
            Ok(()) => self.command_line.push_info(&format!(
                "协同：{name} 已保存为 {shown}，正在连接 {}",
                super::shown_address(&self.collab.server_url)
            )),
            Err(missing) => self
                .command_line
                .push_info(&format!("协同：{name} 已保存为 {shown}；{missing}，暂不连接")),
        }
        self.refresh_collab_view();
    }

    /// Start the session the saved settings describe, on the drawing in
    /// front — or say what is still missing.
    ///
    /// Only for a run that has none: `server_url` going from empty to an
    /// address is what registers the transport's subscription, and the
    /// worker reads the published config as it starts.
    fn start_saved_session(&mut self) -> Result<(), String> {
        let server = self.collab.configured_server_url.trim().to_owned();
        if server.is_empty() {
            return Err("还没有服务器地址（PIDCOLLAB SERVER <地址>）".to_owned());
        }
        if self.collab.user_id.trim().is_empty() {
            return Err("还没有指定用户（PIDCOLLAB USER <用户>）".to_owned());
        }
        let i = self.active_tab;
        let Some(tab) = self.tabs.get(i).filter(|tab| !tab.is_start) else {
            return Err("还没有打开要协同的图纸".to_owned());
        };
        if self.collab.drawing_id.trim().is_empty() {
            // As at start-up: a drawing is known by its file name.
            let stem = tab
                .current_path
                .as_ref()
                .and_then(|path| path.file_stem())
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default();
            if stem.is_empty() {
                return Err(
                    "当前图纸还没有文件名，用 PIDCOLLAB DRAWING <图号> 指定图纸".to_owned(),
                );
            }
            self.collab.drawing_id = stem;
        }
        self.collab.document = Some(tab.id);
        self.collab.server_url = server;
        #[cfg(not(target_arch = "wasm32"))]
        self.collab.sync_client_config();
        Ok(())
    }

    /// A panel interaction: claim, release, or expand the claims list.
    pub(in crate::app) fn on_pid_collab(&mut self, msg: PidCollabMsg) -> Task<Message> {
        match msg {
            PidCollabMsg::ToggleClaims => {
                self.pid_collab_claims_open = !self.pid_collab_claims_open;
                self.refresh_collab_view();
                Task::none()
            }
            PidCollabMsg::Claim(tag) => self.on_collab_claim(&tag, true),
            PidCollabMsg::Release(tag) => self.on_collab_claim(&tag, false),
        }
    }

    /// Queue one `claim` / `release` frame for `tag`, or say why it cannot go.
    ///
    /// The server's answer arrives later as a `CollabEvent`; this only puts the
    /// request on the transport's outbound queue, so the panel's click never
    /// blocks the UI thread on a socket write.
    pub(in crate::app) fn on_collab_claim(&mut self, tag: &str, claim: bool) -> Task<Message> {
        let tag = self.canonical_tag(tag);
        let tag = tag.as_str();
        if tag.is_empty() {
            self.command_line
                .push_error("协同：位号为空，未发起占用请求。");
            return Task::none();
        }
        let outcome = if claim {
            self.collab.request_claim(tag)
        } else {
            self.collab.request_release(tag)
        };
        match outcome {
            Ok(()) => {
                let verb = if claim { "占用" } else { "释放" };
                self.command_line
                    .push_info(&format!("协同：已请求{verb}位号 {tag} …"));
            }
            Err(reason) => {
                self.command_line
                    .push_error(&format!("协同：位号 {tag} 请求未发送（{reason}）。"));
            }
        }
        Task::none()
    }

    /// A tag as the session's drawing spells it.
    ///
    /// On the wire a tag is the text the drawing stores: that is what an
    /// `object_ref` is compared as, and what the server keys a claim by. What
    /// a person types is looked up to that text first, the way `PIDTAG`
    /// finds the tag it is to change: the same letters in another case are
    /// that tag, when the drawing — or, for a release, the claims this
    /// session knows of — has exactly one spelling of them. Typed as
    /// `p-1001`, a claim used to be for a tag nothing carries, beside the
    /// claim somebody else holds on `P-1001`.
    ///
    /// With no such tag here, or with two spellings, what was typed stands.
    pub(in crate::app) fn canonical_tag(&mut self, typed: &str) -> String {
        let typed = typed.trim();
        let mut spellings: Vec<String> = Vec::new();
        let mut note = |tag: &str| {
            let tag = tag.trim();
            if tag.eq_ignore_ascii_case(typed) && !spellings.iter().any(|known| known == tag) {
                spellings.push(tag.to_owned());
            }
        };
        for tag in self.collab.claims.keys() {
            note(tag);
        }
        if let Some(i) = self.collab_tab() {
            let document = &self.tabs[i].scene.document;
            for entity in document.entities() {
                if let Some(author) =
                    crate::pid::persist::PidAuthor::load(document, entity.common().handle)
                {
                    note(&author.tag);
                }
            }
        }
        match spellings.as_slice() {
            [only] if only != typed => only.clone(),
            _ => typed.to_owned(),
        }
    }

    /// `PIDCLAIM <tag>` / `PIDRELEASE <tag>`.
    ///
    /// Both are one-shot: they queue the frame and report, so there is nothing
    /// to install as an active command.
    pub(in crate::app) fn dispatch_pidclaim(&mut self, cmd: &str, _i: usize) -> Option<Task<Message>> {
        let (verb, rest) = split_verb(cmd)?;
        let claim = match verb {
            "PIDCLAIM" => true,
            "PIDRELEASE" => false,
            _ => return None,
        };
        let tag = rest.trim();
        if tag.is_empty() {
            let name = if claim { "PIDCLAIM" } else { "PIDRELEASE" };
            self.command_line
                .push_info(&format!("{name}  位号 <P-1001>:"));
            return Some(Task::none());
        }
        // A tag with a space would be two words; P&ID tags never are, so the
        // whole remainder is the tag and a second word is a typo worth naming.
        if tag.split_whitespace().count() > 1 {
            self.command_line.push_error(&format!(
                "{}: 只接受一个位号，实得 \"{tag}\"。",
                if claim { "PIDCLAIM" } else { "PIDRELEASE" }
            ));
            return Some(Task::none());
        }
        Some(self.on_collab_claim(tag, claim))
    }
}

/// One of the three things `PIDCOLLAB` keeps in the settings file.
#[derive(Clone, Copy)]
enum CollabSetting {
    Server,
    User,
    Drawing,
}

impl CollabSetting {
    /// The word the command takes it under.
    fn name(self) -> &'static str {
        match self {
            CollabSetting::Server => "SERVER",
            CollabSetting::User => "USER",
            CollabSetting::Drawing => "DRAWING",
        }
    }
}

/// Whether `address` is one the transport can dial: a `ws://` URL naming a
/// host. `wss://` is not — the transport speaks over a plain socket — and is
/// better refused here than saved and retried for ever.
///
/// Nor is one carrying a user name or a password. The protocol does not use
/// them, and the settings file this goes into is plain text. What was typed
/// is not echoed in that refusal.
fn dialable(address: &str) -> Result<(), String> {
    if address.contains(char::is_whitespace) {
        return Err("地址里有空白".to_owned());
    }
    let parsed = url::Url::parse(address).map_err(|error| format!("地址无效（{error}）"))?;
    if parsed.scheme() != "ws" {
        return Err(format!("不支持 {}:// 的地址", parsed.scheme()));
    }
    if parsed.host_str().is_none_or(str::is_empty) {
        return Err("地址缺少主机名".to_owned());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("地址里不要带用户名或口令（协同协议不用它们，设置文件是明文）".to_owned());
    }
    Ok(())
}

/// Split a command line into its verb and the remainder, or `None` when the
/// verb does not match a claim command at all.
fn split_verb(cmd: &str) -> Option<(&str, &str)> {
    let trimmed = cmd.trim_start();
    let end = trimmed
        .find(char::is_whitespace)
        .unwrap_or(trimmed.len());
    let verb = &trimmed[..end];
    let verb = if verb.eq_ignore_ascii_case("PIDCLAIM") {
        "PIDCLAIM"
    } else if verb.eq_ignore_ascii_case("PIDRELEASE") {
        "PIDRELEASE"
    } else {
        return None;
    };
    Some((verb, &trimmed[end..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_claim_command_splits_into_verb_and_tag() {
        assert_eq!(
            split_verb("PIDCLAIM P-1001"),
            Some(("PIDCLAIM", " P-1001"))
        );
        assert_eq!(
            split_verb("pidrelease  P-1001"),
            Some(("PIDRELEASE", "  P-1001"))
        );
        assert_eq!(split_verb("PIDCLAIM"), Some(("PIDCLAIM", "")));
    }

    #[test]
    fn another_command_is_not_a_claim_command() {
        assert_eq!(split_verb("PIDTAG P-1001 P-1009"), None);
        assert_eq!(split_verb("PIDCLAIMX P-1001"), None);
        assert_eq!(split_verb(""), None);
    }

    /// The panel is a projection of the session state: a `welcome` that already
    /// lists a claim shows up in the panel with its holder, and the row offers
    /// "release" only for this session's own tags.
    #[test]
    fn the_panel_view_projects_the_session_state() {
        use crate::app::collab::{CollabEvent, PeerInfo};
        use crate::app::OpenCADStudio;

        let mut app = OpenCADStudio::new_for_test();
        app.automation_op(r#"{"op":"new"}"#);
        app.collab.server_url = "ws://127.0.0.1:3240/ws".to_owned();
        app.collab.user_id = "u1".to_owned();
        app.collab.apply(CollabEvent::Connected {
            session_id: "s-1".to_owned(),
            base_revision: 0,
            repo_id: "r-test".into(),
            // The others on the drawing: the welcome does not list the
            // session it welcomes.
            peers: vec![PeerInfo {
                session_id: "s-2".to_owned(),
                user_id: "u2".to_owned(),
                display_name: "工艺乙".to_owned(),
            }],
            claims: vec![("P-1001".to_owned(), "u2".to_owned())],
        });
        app.refresh_collab_view();

        let view = &app.pid_collab_view;
        assert!(view.connected);
        // The panel's roster is everybody: this session's own row first,
        // then the others.
        assert_eq!(view.peers.len(), 2);
        assert!(view.peers[0].is_self, "本机会话必须标记");
        assert_eq!(view.peers[0].user_id, "u1");
        assert!(!view.peers[1].is_self);
        assert_eq!(view.peers[1].user_id, "u2");
        let row = view
            .claims
            .iter()
            .find(|claim| claim.tag == "P-1001")
            .expect("占用行必须出现在面板里");
        assert_eq!(row.holder, "u2");
        assert!(!row.is_self, "别人的占用行不能提供「释放」");
        assert_eq!(view.remote_tags(), vec!["P-1001".to_string()]);
    }

    /// An app in a session of three: `u1` twice (`s-1`, `s-3`) and `u2`
    /// (`s-2`), seen from `u2`.
    fn app_as_u2_among_three() -> crate::app::OpenCADStudio {
        use crate::app::collab::{CollabEvent, PeerInfo};

        let mut app = crate::app::OpenCADStudio::new_for_test();
        app.automation_op(r#"{"op":"new"}"#);
        app.collab.server_url = "ws://127.0.0.1:3240/ws".to_owned();
        app.collab.user_id = "u2".to_owned();
        let peer = |session: &str, user: &str| PeerInfo {
            session_id: session.to_owned(),
            user_id: user.to_owned(),
            display_name: user.to_owned(),
        };
        app.collab.apply(CollabEvent::Connected {
            session_id: "s-2".to_owned(),
            base_revision: 0,
            repo_id: "r-test".into(),
            peers: vec![peer("s-1", "u1"), peer("s-3", "u1")],
            claims: Vec::new(),
        });
        app.refresh_collab_view();
        app
    }

    /// VAL-CLI-012: a downgraded session's panel says it is read-only and
    /// marks who holds the edit right — the holding *session*, so the same
    /// user's other session is not marked along with it.
    #[test]
    fn a_downgrade_marks_the_holding_session_in_the_panel() {
        use crate::app::collab::CollabEvent;

        let mut app = app_as_u2_among_three();
        assert!(!app.pid_collab_view.readonly);
        assert!(app.pid_collab_view.peers.iter().all(|peer| !peer.is_holder));

        app.collab.apply(CollabEvent::ReadonlyDowngrade {
            reason: "图纸已切换为只读".to_owned(),
            holder: Some("s-1".to_owned()),
            level: None,
            code: None,
        });
        app.refresh_collab_view();

        let view = &app.pid_collab_view;
        assert!(view.readonly);
        let marked: Vec<(&str, bool, bool)> = view
            .peers
            .iter()
            .map(|peer| (peer.user_id.as_str(), peer.is_self, peer.is_holder))
            .collect();
        assert_eq!(
            marked,
            [("u2", true, false), ("u1", false, true), ("u1", false, false)]
        );
    }

    /// A session that reconnects is no longer downgraded, so the badge and the
    /// marker stop being drawn — and must stop reporting where they were.
    #[test]
    fn a_reconnect_stops_the_readonly_badge_reporting_its_rectangle() {
        use crate::app::collab::{CollabEvent, PeerInfo};
        use crate::ui::window::pid_collab::{
            holder_bounds_id, HOLDER_BOUNDS_PREFIX, READONLY_BOUNDS_ID,
        };

        crate::ui::wrap_bar::forget_bounds("pid-collab-");
        let mut app = app_as_u2_among_three();
        app.collab.apply(CollabEvent::ReadonlyDowngrade {
            reason: "图纸已切换为只读".to_owned(),
            holder: Some("s-1".to_owned()),
            level: None,
            code: None,
        });
        app.refresh_collab_view();
        let drawn = iced::Rectangle {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        };
        crate::ui::wrap_bar::set_drawn_bounds(READONLY_BOUNDS_ID, drawn);
        crate::ui::wrap_bar::set_drawn_bounds(&holder_bounds_id("u1"), drawn);

        app.collab.apply(CollabEvent::Connected {
            session_id: "s-9".to_owned(),
            base_revision: 0,
            repo_id: "r-test".into(),
            peers: vec![PeerInfo {
                session_id: "s-9".to_owned(),
                user_id: "u2".to_owned(),
                display_name: "u2".to_owned(),
            }],
            claims: Vec::new(),
        });
        app.refresh_collab_view();

        assert!(!app.pid_collab_view.readonly);
        let left: Vec<String> = crate::ui::wrap_bar::recorded_bounds()
            .into_iter()
            .map(|(id, _)| id)
            .filter(|id| id == READONLY_BOUNDS_ID || id.starts_with(HOLDER_BOUNDS_PREFIX))
            .collect();
        assert!(left.is_empty(), "{left:?}");
    }

    /// VAL-CLI-008: the claimed tag draws in the claim colour on the entities
    /// that carry it, and an unclaimed tag is left alone.
    #[test]
    fn a_claimed_tag_colours_the_entities_that_carry_it() {
        use crate::app::collab::CollabEvent;
        use crate::app::OpenCADStudio;
        use crate::pid::persist::PidAuthor;
        use crate::scene::CLAIM_COLOR;
        use document::entities::Insert;
        use document::types::Vector3;

        let mut app = OpenCADStudio::new_for_test();
        app.automation_op(r#"{"op":"new"}"#);
        app.collab.user_id = "u1".to_owned();

        let i = app.active_tab;
        let (claimed, other) = {
            let document = &mut app.tabs[i].scene.document;
            let claimed = document
                .add_entity(document::EntityType::Insert(Insert::new(
                    "PID_SYM_equipment_vessel",
                    Vector3::ZERO,
                )))
                .unwrap();
            let other = document
                .add_entity(document::EntityType::Insert(Insert::new(
                    "PID_SYM_equipment_vessel",
                    Vector3::new(40.0, 0.0, 0.0),
                )))
                .unwrap();
            PidAuthor {
                type_code: "EQUIPMENT".to_owned(),
                tag: "P-1001".to_owned(),
                tag_source: "auto".to_owned(),
                tag_status: "ok".to_owned(),
                ..PidAuthor::default()
            }
            .store(document, claimed);
            PidAuthor {
                type_code: "EQUIPMENT".to_owned(),
                tag: "P-1002".to_owned(),
                tag_source: "auto".to_owned(),
                tag_status: "ok".to_owned(),
                ..PidAuthor::default()
            }
            .store(document, other);
            (claimed, other)
        };

        // u2 holds P-1001: that symbol draws in the claim colour.
        app.collab.apply(CollabEvent::ClaimDenied {
            tag: "P-1001".to_owned(),
            holder: "u2".to_owned(),
            expires_at: 1_700_000_300,
        });
        app.sync_claim_colors();
        let display = app.tabs[i]
            .scene
            .claim_display
            .as_ref()
            .expect("占用必须着色");
        assert_eq!(display.color, CLAIM_COLOR);
        assert!(display.handles.contains(&claimed));
        assert!(!display.handles.contains(&other));

        // A release drops the colouring entirely.
        app.collab.apply(CollabEvent::ClaimReleased {
            tag: "P-1001".to_owned(),
            reason: "expired".to_owned(),
        });
        app.sync_claim_colors();
        assert!(app.tabs[i].scene.claim_display.is_none());
    }

    /// This session's own claim colours the drawing too: the tint marks the
    /// entity as occupied, which is what `VAL-CLI-008` reads off `u1`'s symbol.
    #[test]
    fn this_sessions_own_claim_is_coloured_as_occupied() {
        use crate::app::collab::CollabEvent;
        use crate::app::OpenCADStudio;
        use crate::pid::persist::PidAuthor;
        use document::entities::Insert;
        use document::types::Vector3;

        let mut app = OpenCADStudio::new_for_test();
        app.automation_op(r#"{"op":"new"}"#);
        app.collab.user_id = "u1".to_owned();

        let i = app.active_tab;
        let claimed = {
            let document = &mut app.tabs[i].scene.document;
            let claimed = document
                .add_entity(document::EntityType::Insert(Insert::new(
                    "PID_SYM_equipment_vessel",
                    Vector3::ZERO,
                )))
                .unwrap();
            PidAuthor {
                type_code: "EQUIPMENT".to_owned(),
                tag: "P-1001".to_owned(),
                tag_source: "auto".to_owned(),
                tag_status: "ok".to_owned(),
                ..PidAuthor::default()
            }
            .store(document, claimed);
            claimed
        };

        app.collab.apply(CollabEvent::ClaimGranted {
            tag: "P-1001".to_owned(),
        });
        app.sync_claim_colors();
        let display = app.tabs[i]
            .scene
            .claim_display
            .as_ref()
            .expect("自己占用也要显示占用色");
        assert!(display.handles.contains(&claimed));
    }

    /// A claim row that stops being drawn must stop reporting its old
    /// rectangle: a bounds reader (the REST `ui_bounds` op) would otherwise
    /// find a row for a claim the panel already dropped, and `VAL-CLI-008`
    /// reads exactly that rectangle to locate the row it screenshots.
    #[test]
    fn a_released_claim_row_stops_reporting_its_rectangle() {
        use crate::app::collab::CollabEvent;
        use crate::app::OpenCADStudio;
        use crate::ui::window::pid_collab::ROW_BOUNDS_PREFIX;

        crate::ui::wrap_bar::forget_bounds(ROW_BOUNDS_PREFIX);
        let mut app = OpenCADStudio::new_for_test();
        app.automation_op(r#"{"op":"new"}"#);
        app.collab.server_url = "ws://127.0.0.1:3240/ws".to_owned();
        app.collab.user_id = "u1".to_owned();
        app.collab.apply(CollabEvent::ClaimGranted {
            tag: "P-1001".to_owned(),
        });
        app.refresh_collab_view();
        crate::ui::wrap_bar::set_drawn_bounds(
            &crate::ui::window::pid_collab::row_bounds_id("P-1001"),
            iced::Rectangle {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            },
        );
        assert!(crate::ui::wrap_bar::recorded_bounds()
            .iter()
            .any(|(id, _)| id == "pid-collab-row-P-1001"));

        app.collab.apply(CollabEvent::ClaimReleased {
            tag: "P-1001".to_owned(),
            reason: "released".to_owned(),
        });
        app.refresh_collab_view();
        assert!(
            !crate::ui::wrap_bar::recorded_bounds()
                .iter()
                .any(|(id, _)| id.starts_with(ROW_BOUNDS_PREFIX)),
            "释放后占用行不应再报告矩形"
        );
    }

    /// The first live session brings the panel out once, so a claim is visible
    /// without typing `PIDCOLLAB`; closing it by hand stays closed, because a
    /// reconnect must not reopen a panel the person dismissed.
    #[test]
    fn the_first_session_shows_the_panel_only_once() {
        use crate::app::collab::CollabEvent;
        use crate::app::OpenCADStudio;
        use crate::ui::dock::PanelId;

        let mut app = OpenCADStudio::new_for_test();
        app.automation_op(r#"{"op":"new"}"#);
        app.collab.server_url = "ws://127.0.0.1:3240/ws".to_owned();
        assert!(!app.show_pid_collab);

        let _ = app.update(crate::app::Message::CollabEvent(CollabEvent::Connected {
            session_id: "s-1".to_owned(),
            base_revision: 0,
            repo_id: "r-test".into(),
            peers: Vec::new(),
            claims: Vec::new(),
        }));
        assert!(app.show_pid_collab, "首次连上应把协同面板显示出来");
        assert!(app.dock.location(PanelId::PidCollab).is_some());

        // The person closes it; a later reconnect leaves it closed.
        app.show_pid_collab = false;
        let _ = app.update(crate::app::Message::CollabEvent(CollabEvent::Connected {
            session_id: "s-2".to_owned(),
            base_revision: 0,
            repo_id: "r-test".into(),
            peers: Vec::new(),
            claims: Vec::new(),
        }));
        assert!(!app.show_pid_collab, "重连不应重新打开已关闭的面板");
    }

    /// A tag with no entity in this drawing colours nothing: the panel still
    /// lists the claim, but there is no symbol to tint.
    #[test]
    fn a_claim_on_a_tag_this_drawing_lacks_colours_nothing() {
        use crate::app::collab::CollabEvent;
        use crate::app::OpenCADStudio;

        let mut app = OpenCADStudio::new_for_test();
        app.automation_op(r#"{"op":"new"}"#);
        app.collab.apply(CollabEvent::ClaimGranted {
            tag: "P-9999".to_owned(),
        });
        app.sync_claim_colors();
        assert!(app.tabs[app.active_tab].scene.claim_display.is_none());
    }

    fn app_with_tags(tags: &[&str]) -> crate::app::OpenCADStudio {
        use crate::pid::persist::PidAuthor;
        use document::entities::Insert;
        use document::types::Vector3;

        let mut app = crate::app::OpenCADStudio::new_for_test();
        app.automation_op(r#"{"op":"new"}"#);
        let i = app.active_tab;
        let document = &mut app.tabs[i].scene.document;
        for (n, tag) in tags.iter().enumerate() {
            let handle = document
                .add_entity(document::EntityType::Insert(Insert::new(
                    "PID_SYM_equipment_vessel",
                    Vector3::new(40.0 * n as f64, 0.0, 0.0),
                )))
                .unwrap();
            PidAuthor {
                type_code: "EQUIPMENT".to_owned(),
                tag: (*tag).to_owned(),
                tag_source: "auto".to_owned(),
                tag_status: "ok".to_owned(),
                ..PidAuthor::default()
            }
            .store(document, handle);
        }
        app
    }

    /// A claim names a tag as the drawing spells it. Typed in another case it
    /// is still that tag — the claim used to go out for `p-1001`, a tag
    /// nothing carries, beside whatever claim there was on `P-1001`.
    #[test]
    fn a_typed_tag_is_claimed_as_the_drawing_spells_it() {
        let mut app = app_with_tags(&["P-1001", "V-1001"]);
        assert_eq!(app.canonical_tag("P-1001"), "P-1001");
        assert_eq!(app.canonical_tag("p-1001"), "P-1001");
        assert_eq!(app.canonical_tag("  v-1001 "), "V-1001");
        // A tag this drawing does not have stands as typed (less the blanks).
        assert_eq!(app.canonical_tag(" x-9 "), "x-9");
        assert_eq!(app.canonical_tag(""), "");
    }

    /// Two spellings in one drawing are two tags, and neither is guessed at.
    #[test]
    fn a_typed_tag_fitting_two_spellings_stands_as_typed() {
        let mut app = app_with_tags(&["P-1001", "p-1001"]);
        assert_eq!(app.canonical_tag("p-1001"), "p-1001");
        assert_eq!(app.canonical_tag("P-1001"), "P-1001");
        assert_eq!(app.canonical_tag("P-1001 "), "P-1001");
    }

    /// A release names a claim, and the claim may be on a tag the drawing no
    /// longer carries: the claims this session knows of are looked through
    /// as well.
    #[test]
    fn a_release_finds_the_claim_by_the_spelling_it_was_made_under() {
        use crate::app::collab::CollabEvent;

        let mut app = app_with_tags(&["V-1001"]);
        app.collab.apply(CollabEvent::ClaimGranted {
            tag: "P-1001".to_owned(),
        });
        assert_eq!(app.canonical_tag("p-1001"), "P-1001");
    }

    // ── The server, the user and the drawing, named from inside ──────────

    const SERVER: &str = "ws://127.0.0.1:3240/ws";

    fn run(app: &mut crate::app::OpenCADStudio, cmd: &str) -> serde_json::Value {
        app.automation_op(&serde_json::json!({"op": "run", "cmd": cmd}).to_string())
    }

    /// `VAL-CROSS-006`: a person who has not collaborated before names the
    /// server from inside the program. The address goes into what the
    /// settings file is written from, and — this run having no session yet —
    /// the session for the drawing in front starts on it, with no restart.
    #[test]
    fn naming_the_server_saves_it_and_starts_the_session() {
        let mut app = app_with_tags(&["P-1001"]);
        // Who, and on which drawing, as `--user` and `--drawing` leave them.
        app.collab.user_id = "u1".to_owned();
        app.collab.drawing_id = "PID-1101".to_owned();
        assert!(!app.collab.wanted());
        assert!(app.current_config().collab.server_url.is_empty());

        let done = run(&mut app, &format!("PIDCOLLAB SERVER {SERVER}"));
        assert_eq!(done["ok"], true, "{done}");

        assert_eq!(
            app.current_config().collab.server_url,
            SERVER,
            "what settings.json is written from"
        );
        assert!(app.collab.wanted(), "the transport is subscribed to on this");
        let config = app.collab.client_config();
        assert_eq!(
            (config.server_url.as_str(), config.user_id.as_str(), config.drawing_id.as_str()),
            (SERVER, "u1", "PID-1101")
        );
        assert_eq!(
            app.collab.document,
            Some(app.tabs[app.active_tab].id),
            "the session is for the drawing in front when it was started"
        );
        assert!(app.pid_collab_view.configured(), "the panel was not told");
    }

    /// The note an unconfigured panel draws stops reporting where it is once
    /// the panel has a server: a client reading the bounds takes the id's
    /// presence for the state.
    #[test]
    fn a_panel_given_a_server_stops_reporting_the_unconfigured_note() {
        use crate::ui::window::pid_collab::NOT_CONFIGURED_BOUNDS_ID;

        let mut app = app_with_tags(&["P-1001"]);
        app.collab.user_id = "u1".to_owned();
        app.collab.drawing_id = "PID-1101".to_owned();
        app.refresh_collab_view();
        assert!(!app.pid_collab_view.configured());
        // As the panel records it when it is drawn.
        crate::ui::wrap_bar::set_drawn_bounds(
            NOT_CONFIGURED_BOUNDS_ID,
            iced::Rectangle { x: 0.0, y: 0.0, width: 10.0, height: 10.0 },
        );

        let done = run(&mut app, &format!("PIDCOLLAB SERVER {SERVER}"));
        assert_eq!(done["ok"], true, "{done}");
        let left: Vec<String> = crate::ui::wrap_bar::recorded_bounds()
            .into_iter()
            .map(|(id, _)| id)
            .filter(|id| id == NOT_CONFIGURED_BOUNDS_ID)
            .collect();
        assert!(left.is_empty(), "{left:?}");
    }

    /// An address the transport could not dial is refused where it is typed:
    /// nothing is saved and nothing starts.
    #[test]
    fn an_address_that_is_not_a_websocket_url_is_refused() {
        for typed in [
            "127.0.0.1:3240",
            "http://127.0.0.1:3240/ws",
            // No TLS in the transport: an address it cannot dial.
            "wss://example.invalid/ws",
            "ws://",
            "ws://127.0.0.1:3240/ws extra",
        ] {
            let mut app = app_with_tags(&["P-1001"]);
            app.collab.user_id = "u1".to_owned();
            app.collab.drawing_id = "PID-1101".to_owned();
            let refused = run(&mut app, &format!("PIDCOLLAB SERVER {typed}"));
            assert_eq!(refused["ok"], false, "{typed}: {refused}");
            assert!(
                app.current_config().collab.server_url.is_empty(),
                "{typed} was saved"
            );
            assert!(!app.collab.wanted(), "{typed} started a session");
        }
    }

    /// An address carrying a user name and a password is refused — the
    /// protocol has no use for them and the settings file is plain text —
    /// and the refusal does not repeat them.
    #[test]
    fn an_address_carrying_credentials_is_refused_and_not_repeated() {
        let mut app = app_with_tags(&["P-1001"]);
        app.collab.user_id = "u1".to_owned();
        app.collab.drawing_id = "PID-1101".to_owned();
        let refused = run(&mut app, "PIDCOLLAB SERVER ws://someone:hunter2@127.0.0.1:3240/ws");
        assert_eq!(refused["ok"], false, "{refused}");
        let said = refused.to_string();
        assert!(!said.contains("hunter2"), "the password is repeated in {said}");
        assert!(!said.contains("someone"), "the user name is repeated in {said}");
        assert!(said.contains("127.0.0.1:3240"), "which address was refused is not said in {said}");
        assert!(app.current_config().collab.server_url.is_empty());
        assert!(!app.collab.wanted());

        // Nor in the refusal of a word the command does not know.
        let refused = run(&mut app, "PIDCOLLAB SERVE ws://someone:hunter2@127.0.0.1:3240/ws");
        assert_eq!(refused["ok"], false, "{refused}");
        assert!(!refused.to_string().contains("hunter2"), "{refused}");
    }

    /// With nobody to connect as, or no drawing to connect for, the address
    /// is kept and the session waits for what is missing — and says what.
    #[test]
    fn a_session_waits_for_a_user_and_a_drawing() {
        // A new drawing, never saved: it has no file name to be known by.
        let mut app = app_with_tags(&["P-1001"]);
        let saved = run(&mut app, &format!("PIDCOLLAB SERVER {SERVER}"));
        assert_eq!(saved["ok"], true, "{saved}");
        assert_eq!(app.current_config().collab.server_url, SERVER);
        assert!(!app.collab.wanted(), "nobody to connect as");

        let named = run(&mut app, "PIDCOLLAB USER u1");
        assert_eq!(named["ok"], true, "{named}");
        assert_eq!(app.current_config().collab.user_id, "u1");
        assert!(!app.collab.wanted(), "no drawing to connect for");

        let named = run(&mut app, "PIDCOLLAB DRAWING PID-1101");
        assert_eq!(named["ok"], true, "{named}");
        assert_eq!(app.current_config().collab.drawing_id, "PID-1101");
        assert!(app.collab.wanted());
        let config = app.collab.client_config();
        assert_eq!(
            (config.server_url.as_str(), config.user_id.as_str(), config.drawing_id.as_str()),
            (SERVER, "u1", "PID-1101")
        );
        assert_eq!(app.collab.document, Some(app.tabs[app.active_tab].id));
    }

    /// A run that has its session keeps it. Another address is for the next
    /// start: saved, and the live session left where it is.
    #[test]
    fn another_address_during_a_session_is_kept_for_the_next_start() {
        use crate::app::collab::CollabEvent;

        let mut app = app_with_tags(&["P-1001"]);
        // As `--collab` leaves it: live, and not what is saved.
        app.collab.server_url = SERVER.to_owned();
        app.collab.user_id = "u1".to_owned();
        app.collab.drawing_id = "PID-1101".to_owned();
        app.collab.apply(CollabEvent::Connected {
            session_id: "s-1".to_owned(),
            base_revision: 0,
            repo_id: "r-test".into(),
            peers: Vec::new(),
            claims: Vec::new(),
        });

        let other = "ws://10.0.0.9:3240/ws";
        let done = run(&mut app, &format!("PIDCOLLAB SERVER {other}"));
        assert_eq!(done["ok"], true, "{done}");
        assert_eq!(app.current_config().collab.server_url, other);
        assert_eq!(app.collab.server_url, SERVER, "the live session moved");
        assert!(app.collab.connected);

        // Forgetting it is the same: the next start is offline, this run is
        // not cut off.
        let done = run(&mut app, "PIDCOLLAB SERVER OFF");
        assert_eq!(done["ok"], true, "{done}");
        assert!(app.current_config().collab.server_url.is_empty());
        assert_eq!(app.collab.server_url, SERVER);
    }

    /// The forms the command has always had still mean what they meant, and
    /// a word it does not know is refused with the ones it does.
    #[test]
    fn the_panel_forms_stand_and_an_unknown_word_is_refused() {
        let mut app = app_with_tags(&["P-1001"]);
        assert_eq!(run(&mut app, "PIDCOLLAB ON")["ok"], true);
        assert!(app.show_pid_collab);
        assert_eq!(run(&mut app, "PIDCOLLAB OFF")["ok"], true);
        assert!(!app.show_pid_collab);
        assert_eq!(run(&mut app, "PIDCOLLAB")["ok"], true);
        assert!(app.show_pid_collab);

        let refused = run(&mut app, "PIDCOLLAB SERVE ws://127.0.0.1:3240/ws");
        assert_eq!(refused["ok"], false, "{refused}");
        let said = refused.to_string();
        for form in ["ON", "OFF", "SERVER", "USER", "DRAWING"] {
            assert!(said.contains(form), "{form} is not offered in {said}");
        }
        assert!(app.current_config().collab.server_url.is_empty());
    }
}
