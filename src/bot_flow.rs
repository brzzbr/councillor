use std::time;
use async_openai::Client;
use async_openai::config::OpenAIConfig;
use async_openai::types::{ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs, CreateChatCompletionRequestArgs, Role};
use teloxide::Bot;
use teloxide::dispatching::UpdateHandler;
use teloxide::macros::BotCommands;
use teloxide::prelude::*;
use teloxide::types::{BotCommand, ChatMemberKind, InlineKeyboardButton, InlineKeyboardMarkup, MenuButton};

use crate::AppConfig;
use crate::kinda_db::KindaDb;

trait UserName {
    fn get_user_name(&self) -> String;
}

impl UserName for Message {
    fn get_user_name(&self) -> String {
        self.from().map(|u| u.full_name()).unwrap_or("unknown".to_string())
    }
}

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase")]
pub enum Command {
    #[command(description = "начать работу с ботом")]
    Start,
    #[command(description = "начать новый разговор (полезно, чтобы не перегружать бота)")]
    New,
}

type HandlerResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

pub fn schema() -> UpdateHandler<Box<dyn std::error::Error + Send + Sync + 'static>> {
    use dptree::case;

    dptree::entry()
        .branch(Update::filter_my_chat_member().endpoint(chat_member))
        .branch(
            Update::filter_message()
                .filter_command::<Command>()
                .branch(case![Command::Start].endpoint(start))
                .branch(case![Command::New].endpoint(new_chat)),
        )
        .branch(Update::filter_message().endpoint(chat_msg))
        .branch(Update::filter_callback_query().endpoint(admin_callback))
}

pub async fn chat_msg(
    bot: Bot,
    msg: Message,
    db: KindaDb,
    gpt_client: Client<OpenAIConfig>,
) -> HandlerResult {
    let user_name = msg.get_user_name();
    let msg_txt = msg.text().unwrap_or("");
    log::info!("msg from {} {}", user_name, msg.chat.id);

    let is_user_accepted = db.is_accepted(msg.chat.id).await;
    if is_user_accepted {
        let mut chat_prev = db.chat_prev(msg.chat.id).await;

        let mut msgs = vec![
            ChatCompletionRequestSystemMessageArgs::default()
                .content("Ты ассистент и секретарь. Твой основной язык русский. \
                Ты помогаешь вести деловую переписку и искать нужную информацию. \
                Так же ты хороший переводчик и владеешь всеми языками мира. \
                Ты опытен в составлении статей и имеешь широкий кругозор в науках и \
                программировании.")
                .build()?
                .into()
        ];

        let new_request_msg = ChatCompletionRequestUserMessageArgs::default()
            .content(msg_txt)
            .build()?
            .into();

        msgs.append(&mut chat_prev);
        msgs.push(new_request_msg);

        log::info!("building request from {}", msg.chat.id);
        let request = CreateChatCompletionRequestArgs::default()
            .max_tokens(4096u16)
            .model("gpt-4o")
            .messages(msgs)
            .build()?;
        log::info!("request built from {}", msg.chat.id);

        let response = gpt_client.chat().create(request).await?;
        log::info!("got response to {}", msg.chat.id);

        db.add_to_chat(msg.chat.id, Role::User, msg_txt.to_string()).await;
        log::info!("orig msg added to chat {}", msg.chat.id);

        for choice in response.choices {
            let response_txt = choice.message.content.unwrap_or("".to_string());

            while bot
                .send_message(msg.chat.id, response_txt.clone())
                .await
                .is_err() {
                tokio::time::sleep(time::Duration::from_secs(1)).await;
            }

            log::info!("response sent to {}", msg.chat.id);
            db.add_to_chat(msg.chat.id, Role::Assistant, response_txt).await;
            log::info!("response msg added to chat {}", msg.chat.id);
        }
    } else {
        bot.send_message(msg.chat.id, "Ваша заявка ещё не подтверждена").await?;
    }

    Ok(())
}

pub async fn start(bot: Bot, msg: Message, db: KindaDb, app_cfg: AppConfig) -> HandlerResult {
    let user_name = msg.get_user_name();
    log::info!("{} {} joined",user_name,msg.chat.id);

    db.register(msg.chat.id).await;

    let admin_btn_rows = vec![
        InlineKeyboardButton::callback("✅", format!("accept-{}", msg.chat.id)),
        InlineKeyboardButton::callback("❌", format!("decline-{}", msg.chat.id)),
    ];

    bot.send_message(app_cfg.admin_id, format!("New user attempts to register {}", user_name))
        .reply_markup(InlineKeyboardMarkup::new(vec![admin_btn_rows]))
        .await?;

    bot.set_my_commands(vec![BotCommand::new("new", "начать новый разговор (полезно, чтобы не перегружать бота)")]).await?;

    bot.set_chat_menu_button()
        .chat_id(msg.chat.id)
        .menu_button(MenuButton::Commands)
        .await?;

    bot.send_message(msg.chat.id, "Приветствую! Заявка на рассмотрении...").await?;
    Ok(())
}

pub async fn new_chat(bot: Bot, msg: Message, db: KindaDb) -> HandlerResult {
    let is_user_accepted = db.is_accepted(msg.chat.id).await;

    if is_user_accepted {
        let user_name = msg.get_user_name();
        log::info!("{} {} started new chat", user_name, msg.chat.id);
        db.reset_chat(msg.chat.id).await;
        bot.send_message(msg.chat.id, "Советчик к Вашим услугам").await?;
    }

    Ok(())
}

pub async fn admin_callback(
    bot: Bot,
    db: KindaDb,
    cfg: AppConfig,
    q: CallbackQuery,
) -> HandlerResult {
    let chat_id = q.message.unwrap().chat.id;
    if chat_id == cfg.admin_id {
        if let Some(cmd) = &q.data {
            let maybe_cmd_and_chat = cmd
                .split_once('-')
                .and_then(|(cmd, chat_id_str)|
                    chat_id_str.parse::<i64>().ok().map(|chat_id| (cmd, ChatId(chat_id)))
                );

            match maybe_cmd_and_chat {
                Some(("accept", chat_id)) => {
                    db.confirm(chat_id).await;
                    log::info!("{} {} accepted", q.from.full_name(), chat_id);
                    bot.send_message(chat_id, "Заявка одобрена! Советчик к Вашим услугам").await?;
                }
                Some(("decline", chat_id)) => {
                    db.delete(chat_id).await;
                    log::info!("{} {} declined", q.from.full_name(), chat_id);
                    bot.send_message(chat_id, "Заявка отклонена...").await?;
                }
                _ => log::warn!("unexpected callback {}", cmd),
            }
        }
    } else {
        bot.send_message(chat_id, "Not an admin user").await?;
    }

    Ok(())
}

pub async fn chat_member(mmbr: ChatMemberUpdated, db: KindaDb) -> HandlerResult {
    let new_member = mmbr.new_chat_member.clone();

    if new_member.kind != ChatMemberKind::Member {
        log::info!("{} {} left",mmbr.from.full_name(),mmbr.chat.id);
        db.delete(new_member.user.id.into()).await;
    }

    Ok(())
}
