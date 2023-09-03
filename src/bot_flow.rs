use async_openai::Client;
use async_openai::config::OpenAIConfig;
use async_openai::types::{ChatCompletionRequestMessageArgs, CreateChatCompletionRequestArgs, Role};
use teloxide::Bot;
use teloxide::dispatching::UpdateHandler;
use teloxide::macros::BotCommands;
use teloxide::prelude::*;
use teloxide::types::{ChatMemberKind, InlineKeyboardButton, InlineKeyboardMarkup};

use crate::AppConfig;
use crate::kinda_db::KindaDb;

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase")]
pub enum Command {
    Start,
}

type HandlerResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

pub fn schema() -> UpdateHandler<Box<dyn std::error::Error + Send + Sync + 'static>> {
    use dptree::case;

    dptree::entry()
        .branch(Update::filter_my_chat_member().endpoint(chat_member))
        .branch(
            Update::filter_message()
                .filter_command::<Command>()
                .branch(case![Command::Start].endpoint(start)),
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
    let msg_txt = msg.text().unwrap_or("");
    log::info!("msg from {}: {}", msg.chat.id, msg_txt);

    let is_user_accepted = db.is_accepted(msg.chat.id).await;
    if is_user_accepted {
        let mut chat_prev = db.chat_prev(msg.chat.id).await;

        let mut msgs = vec![
            ChatCompletionRequestMessageArgs::default()
                .role(Role::System)
                .content("Ты канцелярский ассистент и секретарь. \
                Ты помогаешь вести деловую переписку на русском языке, составлять статьи и искать нужную информацию. \
                Так же ты хороший переводчик и владеешь всеми языками мира.")
                .build()?
        ];

        let new_request_msg = ChatCompletionRequestMessageArgs::default()
            .role(Role::User)
            .content(msg_txt)
            .build()?;

        msgs.append(&mut chat_prev);
        msgs.push(new_request_msg);

        let request = CreateChatCompletionRequestArgs::default()
            .max_tokens(1024u16)
            .model("gpt-4")
            .messages(msgs)
            .build()?;

        let response = gpt_client.chat().create(request).await?;
        db.add_to_chat(msg.chat.id, Role::User, msg_txt.to_string()).await;

        for choice in response.choices {
            let response_txt = choice.message.content.unwrap_or("".to_string());
            let _ = bot.send_message(msg.chat.id, response_txt.clone()).await;
            db.add_to_chat(msg.chat.id, Role::Assistant, response_txt).await;
        }
    } else {
        bot.send_message(msg.chat.id, "Ваша заявка ещё не подтверждена").await?;
    }

    Ok(())
}

pub async fn start(bot: Bot, msg: Message, db: KindaDb, app_cfg: AppConfig) -> HandlerResult {
    let user_name = msg.from().map(|u| u.full_name()).unwrap_or("unknown".to_string());
    log::info!("{} {} joined",user_name,msg.chat.id);

    db.register(msg.chat.id).await;

    let admin_btn_rows = vec![
        InlineKeyboardButton::callback("✅", format!("accept-{}", msg.chat.id)),
        InlineKeyboardButton::callback("❌", format!("decline-{}", msg.chat.id)),
    ];

    bot.send_message(app_cfg.admin_id, format!("New user attempts to register {}", user_name))
        .reply_markup(InlineKeyboardMarkup::new(vec![admin_btn_rows]))
        .await?;

    bot.send_message(msg.chat.id, "Приветствую! Заявка на рассмотрении...").await?;
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
                    bot.send_message(
                        chat_id,
                        "Заявка одобрена! Советчик к Вашим услугам",
                    ).await?;
                }
                Some(("decline", chat_id)) => {
                    db.delete(chat_id).await;
                    bot.send_message(
                        chat_id,
                        "Заявка отклонена...",
                    ).await?;
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
        log::info!(
            "user {} {} left",
            new_member.user.full_name(),
            new_member.user.id
        );
        db.delete(new_member.user.id.into()).await;
    }

    Ok(())
}
