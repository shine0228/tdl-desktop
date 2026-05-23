use std::{
    fs,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use chrono::{TimeZone, Utc};
use tauri::{AppHandle, Emitter, State};
use tdlib_rs::{
    enums::{
        AuthorizationState, Chat as TdChat, ChatList, ChatType, Chats as TdChats, ConnectionState,
        MessageContent, Messages as TdMessages, Update, User as TdUser,
    },
    functions, types as td_types,
};

use crate::{
    state::AppState,
    types::{AppConfig, MediaKind, MessageInfo, TgLiteChat, TgLiteEvent, TgLiteStatus},
    util::lock,
};

#[derive(Default)]
pub struct TgLiteRuntime {
    inner: Arc<Mutex<TgLiteInner>>,
}

#[derive(Default)]
struct TgLiteInner {
    client_id: Option<i32>,
    receiver_started: bool,
    run_flag: Option<Arc<AtomicBool>>,
    auth_state: Option<AuthorizationState>,
    qr_link: Option<String>,
    username: Option<String>,
    display_name: Option<String>,
    last_error: Option<String>,
}

struct TgLiteConfig {
    api_id: i32,
    api_hash: String,
    database_directory: String,
    files_directory: String,
}

#[tauri::command]
pub fn tg_lite_status(
    app_state: State<'_, AppState>,
    runtime: State<'_, TgLiteRuntime>,
) -> Result<TgLiteStatus, String> {
    runtime.status(&app_state)
}

#[tauri::command]
pub async fn tg_lite_start(
    app: AppHandle,
    app_state: State<'_, AppState>,
    runtime: State<'_, TgLiteRuntime>,
) -> Result<TgLiteStatus, String> {
    let (client_id, config) = runtime.ensure_client(&app_state, Some(app)).await?;
    runtime.drive_authorization(client_id, &config).await?;
    runtime.refresh_me_if_ready(client_id).await;
    runtime.status(&app_state)
}

#[tauri::command]
pub async fn tg_lite_set_phone(
    phone_number: String,
    app_state: State<'_, AppState>,
    runtime: State<'_, TgLiteRuntime>,
) -> Result<TgLiteStatus, String> {
    let phone_number = phone_number.trim();
    if phone_number.is_empty() {
        return Err("请输入手机号。".into());
    }

    let (client_id, config) = runtime.ensure_client(&app_state, None).await?;
    runtime.drive_authorization(client_id, &config).await?;
    functions::set_authentication_phone_number(phone_number.to_string(), None, client_id)
        .await
        .map_err(td_error)?;
    runtime.wait_for_auth_update(Duration::from_secs(5));
    runtime.status(&app_state)
}

#[tauri::command]
pub async fn tg_lite_request_qr(
    app_state: State<'_, AppState>,
    runtime: State<'_, TgLiteRuntime>,
) -> Result<TgLiteStatus, String> {
    let (client_id, config) = runtime.ensure_client(&app_state, None).await?;
    runtime.drive_authorization(client_id, &config).await?;
    functions::request_qr_code_authentication(Vec::new(), client_id)
        .await
        .map_err(td_error)?;
    runtime.wait_for_auth_update(Duration::from_secs(5));
    runtime.status(&app_state)
}

#[tauri::command]
pub async fn tg_lite_check_code(
    code: String,
    app_state: State<'_, AppState>,
    runtime: State<'_, TgLiteRuntime>,
) -> Result<TgLiteStatus, String> {
    let code = code.trim();
    if code.is_empty() {
        return Err("请输入验证码。".into());
    }

    let (client_id, config) = runtime.ensure_client(&app_state, None).await?;
    runtime.drive_authorization(client_id, &config).await?;
    functions::check_authentication_code(code.to_string(), client_id)
        .await
        .map_err(td_error)?;
    runtime.wait_for_auth_update(Duration::from_secs(5));
    runtime.refresh_me_if_ready(client_id).await;
    runtime.status(&app_state)
}

#[tauri::command]
pub async fn tg_lite_check_password(
    password: String,
    app_state: State<'_, AppState>,
    runtime: State<'_, TgLiteRuntime>,
) -> Result<TgLiteStatus, String> {
    if password.is_empty() {
        return Err("请输入二步验证密码。".into());
    }

    let (client_id, config) = runtime.ensure_client(&app_state, None).await?;
    runtime.drive_authorization(client_id, &config).await?;
    functions::check_authentication_password(password, client_id)
        .await
        .map_err(td_error)?;
    runtime.wait_for_auth_update(Duration::from_secs(5));
    runtime.refresh_me_if_ready(client_id).await;
    runtime.status(&app_state)
}

#[tauri::command]
pub async fn tg_lite_load_chats(
    limit: Option<i32>,
    app_state: State<'_, AppState>,
    runtime: State<'_, TgLiteRuntime>,
) -> Result<Vec<TgLiteChat>, String> {
    let client_id = runtime.ready_client(&app_state).await?;
    let limit = limit.unwrap_or(80).clamp(1, 200);

    let _ = functions::load_chats(Some(ChatList::Main), limit, client_id).await;
    let chats = functions::get_chats(Some(ChatList::Main), limit, client_id)
        .await
        .map_err(td_error)?;

    let TdChats::Chats(chats) = chats;
    let mut result = Vec::with_capacity(chats.chat_ids.len());
    for chat_id in chats.chat_ids {
        let Ok(TdChat::Chat(chat)) = functions::get_chat(chat_id, client_id).await else {
            continue;
        };
        result.push(tg_chat_from_td(chat));
    }
    Ok(result)
}

#[tauri::command]
pub async fn tg_lite_load_messages(
    chat_id: i64,
    limit: Option<i32>,
    app_state: State<'_, AppState>,
    runtime: State<'_, TgLiteRuntime>,
) -> Result<Vec<MessageInfo>, String> {
    let client_id = runtime.ready_client(&app_state).await?;
    let limit = limit.unwrap_or(50).clamp(1, 200);
    let messages = functions::get_chat_history(chat_id, 0, 0, limit, false, client_id)
        .await
        .map_err(td_error)?;

    let TdMessages::Messages(messages) = messages;
    Ok(messages
        .messages
        .into_iter()
        .flatten()
        .map(message_info_from_td)
        .collect())
}

impl TgLiteRuntime {
    async fn ensure_client(
        &self,
        app_state: &AppState,
        app: Option<AppHandle>,
    ) -> Result<(i32, TgLiteConfig), String> {
        let config = tg_lite_config(app_state)?;
        self.start_receiver(app)?;

        if let Some(client_id) = self.active_client_id()? {
            return Ok((client_id, config));
        }

        fs::create_dir_all(&config.database_directory)
            .map_err(|error| format!("无法创建 TDLib 数据目录: {error}"))?;
        fs::create_dir_all(&config.files_directory)
            .map_err(|error| format!("无法创建 TDLib 文件目录: {error}"))?;

        let client_id = tdlib_rs::create_client();
        {
            let mut inner = lock(self.inner.as_ref())?;
            inner.client_id = Some(client_id);
            inner.auth_state = None;
            inner.qr_link = None;
            inner.username = None;
            inner.display_name = None;
            inner.last_error = None;
        }

        functions::set_log_verbosity_level(1, client_id)
            .await
            .map_err(td_error)?;
        Ok((client_id, config))
    }

    async fn ready_client(&self, app_state: &AppState) -> Result<i32, String> {
        let (client_id, config) = self.ensure_client(app_state, None).await?;
        self.drive_authorization(client_id, &config).await?;
        self.refresh_me_if_ready(client_id).await;

        if self.is_authorized()? {
            Ok(client_id)
        } else {
            let status = self.status(app_state)?;
            Err(status.message)
        }
    }

    fn active_client_id(&self) -> Result<Option<i32>, String> {
        let inner = lock(self.inner.as_ref())?;
        let closed = matches!(
            inner.auth_state,
            Some(AuthorizationState::Closed | AuthorizationState::Closing)
        );
        Ok(if closed { None } else { inner.client_id })
    }

    fn is_authorized(&self) -> Result<bool, String> {
        let inner = lock(self.inner.as_ref())?;
        Ok(matches!(inner.auth_state, Some(AuthorizationState::Ready)))
    }

    fn start_receiver(&self, app: Option<AppHandle>) -> Result<(), String> {
        {
            let mut inner = lock(self.inner.as_ref())?;
            if inner.receiver_started {
                return Ok(());
            }

            let run_flag = Arc::new(AtomicBool::new(true));
            inner.run_flag = Some(Arc::clone(&run_flag));
            inner.receiver_started = true;

            let inner_ref = Arc::clone(&self.inner);
            thread::Builder::new()
                .name("tg-lite-tdlib-receiver".into())
                .spawn(move || receive_loop(inner_ref, run_flag, app))
                .map_err(|error| format!("启动 TDLib 接收线程失败: {error}"))?;
        }
        Ok(())
    }

    async fn drive_authorization(
        &self,
        client_id: i32,
        config: &TgLiteConfig,
    ) -> Result<(), String> {
        let started = Instant::now();
        let mut sent_parameters = false;

        while started.elapsed() < Duration::from_secs(8) {
            match self.auth_state()? {
                Some(AuthorizationState::WaitTdlibParameters) if !sent_parameters => {
                    sent_parameters = true;
                    functions::set_tdlib_parameters(
                        false,
                        config.database_directory.clone(),
                        config.files_directory.clone(),
                        String::new(),
                        true,
                        true,
                        true,
                        false,
                        config.api_id,
                        config.api_hash.clone(),
                        "zh".into(),
                        "TDL Desktop".into(),
                        std::env::consts::OS.into(),
                        env!("CARGO_PKG_VERSION").into(),
                        client_id,
                    )
                    .await
                    .map_err(td_error)?;
                }
                Some(
                    AuthorizationState::Ready
                    | AuthorizationState::WaitPhoneNumber
                    | AuthorizationState::WaitCode(_)
                    | AuthorizationState::WaitPassword(_)
                    | AuthorizationState::WaitEmailAddress(_)
                    | AuthorizationState::WaitEmailCode(_)
                    | AuthorizationState::WaitOtherDeviceConfirmation(_)
                    | AuthorizationState::WaitRegistration(_)
                    | AuthorizationState::WaitPremiumPurchase(_),
                ) => return Ok(()),
                Some(AuthorizationState::Closed | AuthorizationState::Closing) => {
                    return Err("TDLib 已关闭，请重新启动 TG Lite。".into());
                }
                Some(AuthorizationState::LoggingOut) => {
                    return Err("TDLib 正在退出登录，请稍后重试。".into());
                }
                _ => thread::sleep(Duration::from_millis(150)),
            }
        }

        Ok(())
    }

    fn wait_for_auth_update(&self, timeout: Duration) {
        let started = Instant::now();
        while started.elapsed() < timeout {
            if matches!(
                self.auth_state().ok().flatten(),
                Some(
                    AuthorizationState::Ready
                        | AuthorizationState::WaitCode(_)
                        | AuthorizationState::WaitPassword(_)
                        | AuthorizationState::WaitOtherDeviceConfirmation(_)
                        | AuthorizationState::WaitEmailAddress(_)
                        | AuthorizationState::WaitEmailCode(_)
                        | AuthorizationState::WaitRegistration(_)
                )
            ) {
                return;
            }
            thread::sleep(Duration::from_millis(150));
        }
    }

    async fn refresh_me_if_ready(&self, client_id: i32) {
        if !matches!(
            self.auth_state().ok().flatten(),
            Some(AuthorizationState::Ready)
        ) {
            return;
        }

        let Ok(TdUser::User(user)) = functions::get_me(client_id).await else {
            return;
        };
        let username = user
            .usernames
            .as_ref()
            .and_then(|usernames| usernames.active_usernames.first().cloned())
            .or_else(|| {
                user.usernames.as_ref().and_then(|usernames| {
                    (!usernames.editable_username.is_empty())
                        .then(|| usernames.editable_username.clone())
                })
            });
        let display_name = display_name(&user.first_name, &user.last_name);

        if let Ok(mut inner) = lock(self.inner.as_ref()) {
            inner.username = username;
            inner.display_name = display_name;
        }
    }

    fn auth_state(&self) -> Result<Option<AuthorizationState>, String> {
        Ok(lock(self.inner.as_ref())?.auth_state.clone())
    }

    fn status(&self, app_state: &AppState) -> Result<TgLiteStatus, String> {
        let config = lock(&app_state.config)?;
        let configured = config_is_ready(&config);
        let inner = lock(self.inner.as_ref())?;
        Ok(status_from_inner(configured, &inner))
    }
}

fn receive_loop(inner: Arc<Mutex<TgLiteInner>>, run_flag: Arc<AtomicBool>, app: Option<AppHandle>) {
    while run_flag.load(Ordering::Acquire) {
        match tdlib_rs::receive() {
            Some((update, client_id)) => handle_update(&inner, app.as_ref(), client_id, update),
            None => thread::sleep(Duration::from_millis(25)),
        }
    }
}

fn handle_update(
    inner: &Arc<Mutex<TgLiteInner>>,
    app: Option<&AppHandle>,
    client_id: i32,
    update: Update,
) {
    let Ok(active) = inner.lock().map(|inner| inner.client_id == Some(client_id)) else {
        return;
    };
    if !active {
        return;
    }

    match update {
        Update::AuthorizationState(update) => handle_authorization_update(inner, app, update.authorization_state),
        Update::ConnectionState(update) => emit_tg_lite_event(
            app,
            TgLiteEvent::Connection {
                state: connection_state_label(&update.state).into(),
            },
        ),
        Update::NewChat(update) => emit_tg_lite_event(
            app,
            TgLiteEvent::ChatUpsert {
                chat: tg_chat_from_td(update.chat),
            },
        ),
        Update::ChatLastMessage(update) => emit_chat_update(app, client_id, update.chat_id),
        Update::ChatPosition(update) => {
            if is_main_chat_position(&update.position) {
                if update.position.order == 0 {
                    emit_tg_lite_event(app, TgLiteEvent::ChatDelete { chat_id: update.chat_id });
                } else {
                    emit_chat_update(app, client_id, update.chat_id);
                }
            }
        }
        Update::ChatReadInbox(update) => emit_chat_update(app, client_id, update.chat_id),
        Update::ChatTitle(update) => emit_chat_update(app, client_id, update.chat_id),
        Update::NewMessage(update) => {
            let chat_id = update.message.chat_id;
            emit_tg_lite_event(
                app,
                TgLiteEvent::MessageNew {
                    chat_id,
                    message: message_info_from_td(update.message),
                },
            );
            emit_chat_update(app, client_id, chat_id);
        }
        Update::MessageContent(update) => {
            let mut message = empty_message_info(update.message_id);
            apply_content_to_message(&update.new_content, &mut message);
            emit_tg_lite_event(
                app,
                TgLiteEvent::MessageUpdate {
                    chat_id: update.chat_id,
                    message_id: update.message_id,
                    message: Some(message),
                },
            );
        }
        Update::DeleteMessages(update) => emit_tg_lite_event(
            app,
            TgLiteEvent::MessageDelete {
                chat_id: update.chat_id,
                message_ids: update.message_ids,
            },
        ),
        _ => {}
    }
}

fn handle_authorization_update(
    inner: &Arc<Mutex<TgLiteInner>>,
    app: Option<&AppHandle>,
    auth_state: AuthorizationState,
) {
    let status = if let Ok(mut inner) = inner.lock() {
        inner.qr_link = match &auth_state {
            AuthorizationState::WaitOtherDeviceConfirmation(value) => Some(value.link.clone()),
            AuthorizationState::Ready => None,
            AuthorizationState::WaitPhoneNumber
            | AuthorizationState::WaitCode(_)
            | AuthorizationState::WaitPassword(_)
            | AuthorizationState::WaitEmailAddress(_)
            | AuthorizationState::WaitEmailCode(_)
            | AuthorizationState::WaitRegistration(_)
            | AuthorizationState::WaitTdlibParameters => inner.qr_link.take(),
            _ => inner.qr_link.clone(),
        };
        inner.auth_state = Some(auth_state);
        Some(status_from_inner(true, &inner))
    } else {
        None
    };

    if let Some(status) = status {
        emit_tg_lite_event(app, TgLiteEvent::Status { status });
    }
}

fn emit_chat_update(app: Option<&AppHandle>, client_id: i32, chat_id: i64) {
    let Some(app) = app.cloned() else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        let Ok(TdChat::Chat(chat)) = functions::get_chat(chat_id, client_id).await else {
            return;
        };
        emit_tg_lite_event(
            Some(&app),
            TgLiteEvent::ChatUpsert {
                chat: tg_chat_from_td(chat),
            },
        );
    });
}

fn emit_tg_lite_event(app: Option<&AppHandle>, event: TgLiteEvent) {
    if let Some(app) = app {
        let _ = app.emit("tg-lite-event", event);
    }
}

fn status_from_inner(configured: bool, inner: &TgLiteInner) -> TgLiteStatus {
    let (state, message, qr_link, authorized) = status_parts(
        inner.auth_state.as_ref(),
        inner.qr_link.clone(),
        inner.last_error.as_ref(),
    );

    TgLiteStatus {
        configured,
        initialized: inner.client_id.is_some(),
        authorized,
        state,
        message,
        qr_link,
        username: inner.username.clone(),
        display_name: inner.display_name.clone(),
    }
}

fn tg_lite_config(app_state: &AppState) -> Result<TgLiteConfig, String> {
    let config = lock(&app_state.config)?.clone();
    if !config_is_ready(&config) {
        return Err("请先填写 api_id 和 api_hash。".into());
    }

    let api_id = config
        .tg_lite_api_id
        .trim()
        .parse::<i32>()
        .map_err(|_| "api_id 必须是数字。".to_string())?;
    let api_hash = config.tg_lite_api_hash.trim().to_string();
    let root = app_state.app_dir.join("tg-lite");

    Ok(TgLiteConfig {
        api_id,
        api_hash,
        database_directory: root.join("db").to_string_lossy().to_string(),
        files_directory: root.join("files").to_string_lossy().to_string(),
    })
}

fn config_is_ready(config: &AppConfig) -> bool {
    !config.tg_lite_api_id.trim().is_empty() && !config.tg_lite_api_hash.trim().is_empty()
}

fn status_parts(
    auth_state: Option<&AuthorizationState>,
    qr_link: Option<String>,
    last_error: Option<&String>,
) -> (String, String, Option<String>, bool) {
    if let Some(error) = last_error {
        return ("error".into(), error.clone(), qr_link, false);
    }

    match auth_state {
        None => (
            "notStarted".into(),
            "等待启动 TDLib。".into(),
            qr_link,
            false,
        ),
        Some(AuthorizationState::WaitTdlibParameters) => (
            "initializing".into(),
            "正在初始化 TDLib。".into(),
            qr_link,
            false,
        ),
        Some(AuthorizationState::WaitPhoneNumber) => (
            "waitPhoneNumber".into(),
            "请输入手机号，或使用 QR 登录。".into(),
            qr_link,
            false,
        ),
        Some(AuthorizationState::WaitCode(_)) => (
            "waitCode".into(),
            "请输入 Telegram 发送的验证码。".into(),
            qr_link,
            false,
        ),
        Some(AuthorizationState::WaitPassword(value)) => {
            let hint = if value.password_hint.is_empty() {
                "请输入二步验证密码。".to_string()
            } else {
                format!("请输入二步验证密码，提示：{}", value.password_hint)
            };
            ("waitPassword".into(), hint, qr_link, false)
        }
        Some(AuthorizationState::WaitOtherDeviceConfirmation(value)) => (
            "waitQr".into(),
            "请在已登录 Telegram 的设备中确认 QR 登录。".into(),
            Some(value.link.clone()),
            false,
        ),
        Some(AuthorizationState::WaitEmailAddress(_)) => (
            "waitEmail".into(),
            "当前账号要求邮箱验证，暂未在 TG Lite UI 中支持。".into(),
            qr_link,
            false,
        ),
        Some(AuthorizationState::WaitEmailCode(_)) => (
            "waitEmailCode".into(),
            "当前账号要求邮箱验证码，暂未在 TG Lite UI 中支持。".into(),
            qr_link,
            false,
        ),
        Some(AuthorizationState::WaitRegistration(_)) => (
            "waitRegistration".into(),
            "当前手机号未注册 Telegram，TG Lite 暂不处理注册流程。".into(),
            qr_link,
            false,
        ),
        Some(AuthorizationState::WaitPremiumPurchase(_)) => (
            "waitPremium".into(),
            "Telegram 要求 Premium 购买验证，TG Lite 暂不支持。".into(),
            qr_link,
            false,
        ),
        Some(AuthorizationState::Ready) => (
            "ready".into(),
            "TDLib 已登录，可以读取对话。".into(),
            None,
            true,
        ),
        Some(AuthorizationState::LoggingOut) => (
            "loggingOut".into(),
            "TDLib 正在退出登录。".into(),
            qr_link,
            false,
        ),
        Some(AuthorizationState::Closing) => {
            ("closing".into(), "TDLib 正在关闭。".into(), qr_link, false)
        }
        Some(AuthorizationState::Closed) => (
            "closed".into(),
            "TDLib 已关闭，请重新启动。".into(),
            qr_link,
            false,
        ),
    }
}

fn tg_chat_from_td(chat: td_types::Chat) -> TgLiteChat {
    let (last_message_id, last_message_text) = chat
        .last_message
        .as_ref()
        .map(|message| (Some(message.id), content_text(&message.content)))
        .unwrap_or((None, None));

    TgLiteChat {
        id: chat.id,
        title: chat.title,
        chat_type: chat_type_label(&chat.r#type).into(),
        unread_count: chat.unread_count,
        last_message_id,
        last_message_text,
        order: main_chat_order(&chat.positions),
    }
}

fn main_chat_order(positions: &[td_types::ChatPosition]) -> Option<String> {
    positions
        .iter()
        .find(|position| matches!(position.list, ChatList::Main))
        .and_then(|position| (position.order > 0).then(|| position.order.to_string()))
}

fn is_main_chat_position(position: &td_types::ChatPosition) -> bool {
    matches!(position.list, ChatList::Main)
}

fn connection_state_label(state: &ConnectionState) -> &'static str {
    match state {
        ConnectionState::WaitingForNetwork => "waitingForNetwork",
        ConnectionState::ConnectingToProxy => "connectingToProxy",
        ConnectionState::Connecting => "connecting",
        ConnectionState::Updating => "updating",
        ConnectionState::Ready => "ready",
    }
}

fn chat_type_label(chat_type: &ChatType) -> &'static str {
    match chat_type {
        ChatType::Private(_) => "private",
        ChatType::BasicGroup(_) => "group",
        ChatType::Supergroup(value) if value.is_channel => "channel",
        ChatType::Supergroup(_) => "supergroup",
        ChatType::Secret(_) => "secret",
    }
}

fn message_info_from_td(message: td_types::Message) -> MessageInfo {
    let mut info = empty_message_info(message.id);
    info.date = timestamp_to_rfc3339(message.date);
    apply_content_to_message(&message.content, &mut info);
    info
}

fn empty_message_info(id: i64) -> MessageInfo {
    MessageInfo {
        id,
        date: None,
        text: None,
        media_kind: MediaKind::None,
        media_type: None,
        mime_type: None,
        file_name: None,
        file_size: None,
        width: None,
        height: None,
        duration: None,
        previewable: false,
    }
}

fn apply_content_to_message(content: &MessageContent, info: &mut MessageInfo) {
    match content {
        MessageContent::MessageText(value) => {
            info.text = non_empty(value.text.text.clone());
        }
        MessageContent::MessagePhoto(value) => {
            info.text = non_empty(value.caption.text.clone());
            info.media_kind = MediaKind::Photo;
            info.media_type = Some("photo".into());
            if let Some(size) = value
                .photo
                .sizes
                .iter()
                .max_by_key(|size| size.width * size.height)
            {
                info.file_name = Some(format!("photo_{}.jpg", info.id));
                info.file_size = file_size(&size.photo);
                info.width = Some(i64::from(size.width));
                info.height = Some(i64::from(size.height));
            }
        }
        MessageContent::MessageVideo(value) => {
            info.text = non_empty(value.caption.text.clone());
            info.media_kind = MediaKind::Video;
            info.media_type = Some("video".into());
            info.mime_type = non_empty(value.video.mime_type.clone());
            info.file_name = non_empty(value.video.file_name.clone())
                .or_else(|| Some(format!("video_{}.mp4", info.id)));
            info.file_size = file_size(&value.video.video);
            info.width = Some(i64::from(value.video.width));
            info.height = Some(i64::from(value.video.height));
            info.duration = Some(i64::from(value.video.duration));
        }
        MessageContent::MessageAnimation(value) => {
            info.text = non_empty(value.caption.text.clone());
            info.media_kind = MediaKind::Video;
            info.media_type = Some("animation".into());
            info.mime_type = non_empty(value.animation.mime_type.clone());
            info.file_name = non_empty(value.animation.file_name.clone());
            info.file_size = file_size(&value.animation.animation);
            info.width = Some(i64::from(value.animation.width));
            info.height = Some(i64::from(value.animation.height));
            info.duration = Some(i64::from(value.animation.duration));
        }
        MessageContent::MessageDocument(value) => {
            info.text = non_empty(value.caption.text.clone());
            info.media_kind = MediaKind::Document;
            info.media_type = Some("document".into());
            info.mime_type = non_empty(value.document.mime_type.clone());
            info.file_name = non_empty(value.document.file_name.clone());
            info.file_size = file_size(&value.document.document);
        }
        MessageContent::MessageAudio(value) => {
            info.text = non_empty(value.caption.text.clone());
            info.media_kind = MediaKind::Audio;
            info.media_type = Some("audio".into());
            info.mime_type = non_empty(value.audio.mime_type.clone());
            info.file_name = non_empty(value.audio.file_name.clone())
                .or_else(|| non_empty(value.audio.title.clone()));
            info.file_size = file_size(&value.audio.audio);
            info.duration = Some(i64::from(value.audio.duration));
        }
        MessageContent::MessageVoiceNote(value) => {
            info.text = non_empty(value.caption.text.clone());
            info.media_kind = MediaKind::Audio;
            info.media_type = Some("voice".into());
            info.file_size = file_size(&value.voice_note.voice);
            info.duration = Some(i64::from(value.voice_note.duration));
        }
        MessageContent::MessageVideoNote(value) => {
            info.media_kind = MediaKind::Video;
            info.media_type = Some("video_note".into());
            info.file_size = file_size(&value.video_note.video);
            info.width = Some(i64::from(value.video_note.length));
            info.height = Some(i64::from(value.video_note.length));
            info.duration = Some(i64::from(value.video_note.duration));
        }
        MessageContent::MessageSticker(value) => {
            info.text = non_empty(value.sticker.emoji.clone());
            info.media_kind = MediaKind::Photo;
            info.media_type = Some("sticker".into());
            info.file_size = file_size(&value.sticker.sticker);
            info.width = Some(i64::from(value.sticker.width));
            info.height = Some(i64::from(value.sticker.height));
        }
        _ => {
            info.text = content_text(content);
            if info.text.is_none() {
                info.media_kind = MediaKind::Unknown;
                info.media_type = Some("unsupported".into());
            }
        }
    }
}

fn content_text(content: &MessageContent) -> Option<String> {
    match content {
        MessageContent::MessageText(value) => non_empty(value.text.text.clone()),
        MessageContent::MessagePhoto(value) => non_empty(value.caption.text.clone()),
        MessageContent::MessageVideo(value) => non_empty(value.caption.text.clone()),
        MessageContent::MessageAnimation(value) => non_empty(value.caption.text.clone()),
        MessageContent::MessageDocument(value) => non_empty(value.caption.text.clone()),
        MessageContent::MessageAudio(value) => non_empty(value.caption.text.clone()),
        MessageContent::MessageVoiceNote(value) => non_empty(value.caption.text.clone()),
        MessageContent::MessageSticker(value) => non_empty(value.sticker.emoji.clone()),
        MessageContent::MessageVideoNote(_) => Some("视频消息".into()),
        MessageContent::MessageExpiredPhoto => Some("已过期图片".into()),
        MessageContent::MessageExpiredVideo => Some("已过期视频".into()),
        _ => None,
    }
}

fn file_size(file: &td_types::File) -> Option<i64> {
    let size = file.size.max(file.expected_size);
    (size > 0).then_some(size)
}

fn timestamp_to_rfc3339(timestamp: i32) -> Option<String> {
    Utc.timestamp_opt(i64::from(timestamp), 0)
        .single()
        .map(|date| date.to_rfc3339())
}

fn non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn display_name(first_name: &str, last_name: &str) -> Option<String> {
    non_empty(format!("{first_name} {last_name}"))
}

fn td_error(error: td_types::Error) -> String {
    if error.message.is_empty() {
        format!("TDLib 错误 {}", error.code)
    } else {
        format!("TDLib 错误 {}: {}", error.code, error.message)
    }
}
