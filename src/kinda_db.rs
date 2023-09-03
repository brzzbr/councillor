use std::collections::HashMap;
use std::path;
use std::sync::Arc;
use std::time::SystemTime;

use async_openai::types::{ChatCompletionRequestMessage, Role};
use serde::{Deserialize, Serialize};
use teloxide::prelude::ChatId;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock;

#[derive(Serialize, Deserialize)]
enum ChatState {
    Unconfirmed,
    Confirmed(u64, Vec<ChatCompletionRequestMessage>),
}

type ConsistentState = Arc<RwLock<HashMap<ChatId, ChatState>>>;

#[derive(Clone)]
pub struct KindaDb {
    path: String,
    state: ConsistentState,
}

impl KindaDb {
    pub async fn register(&self, chat_id: ChatId) {
        let mut state = self.state.write().await;
        state.insert(chat_id, ChatState::Unconfirmed);
        self.save_state(&state).await;
    }

    pub async fn confirm(&self, chat_id: ChatId) {
        let mut state = self.state.write().await;
        let now_sec = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        state.insert(chat_id, ChatState::Confirmed(now_sec, vec![]));
        self.reset_chat(chat_id).await;
        self.save_state(&state).await;
    }

    pub async fn delete(&self, chat_id: ChatId) {
        let mut state = self.state.write().await;
        state.remove(&chat_id);
        self.reset_chat(chat_id).await;
        self.save_state(&state).await;
    }

    pub async fn is_accepted(&self, chat_id: ChatId) -> bool {
        let state = self.state.read().await;
        match state.get(&chat_id) {
            Some(ChatState::Confirmed(_, _)) => true,
            _ => false
        }
    }

    pub async fn chat_prev(&self, chat_id: ChatId) -> Vec<ChatCompletionRequestMessage> {
        let state = self.state.read().await;
        let now_sec = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        match state.get(&chat_id) {
            Some(ChatState::Confirmed(updated, msgs)) if now_sec - updated < 1800 => msgs.clone(),
            Some(ChatState::Confirmed(_, _)) => {
                let mut state = self.state.write().await;
                state.insert(chat_id, ChatState::Confirmed(now_sec, vec![]));
                self.reset_chat(chat_id).await;
                self.save_state(&state).await;
                vec![]
            }
            _ => vec![],
        }
    }

    pub async fn add_to_chat(&self, chat_id: ChatId, role: Role, msg: String) {
        let mut state = self.state.write().await;

        if let Some(ChatState::Confirmed(_, msgs)) = state.get(&chat_id) {
            let chat_path = format!("{}/{}.txt", self.path, chat_id);
            let mut chat_file = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(chat_path)
                .await
                .unwrap();

            let chunk = format!("{}***\n{}***\n", serde_json::to_string(&role).unwrap(), msg);
            chat_file.write_all(chunk.as_bytes()).await.unwrap();

            let now_sec = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs();

            let req_msg = ChatCompletionRequestMessage {
                role,
                content: Some(msg),
                name: None,
                function_call: None,
            };
            let mut new_msgs = msgs.clone();
            new_msgs.push(req_msg);
            let new_state = ChatState::Confirmed(now_sec, new_msgs);
            state.insert(chat_id, new_state);
        }

        self.save_state(&state).await;
    }

    pub async fn reset_chat(&self, chat_id: ChatId) {
        let chat_path = format!("{}/{}.txt", self.path, chat_id);
        let _ = fs::remove_file(chat_path).await;
    }

    pub async fn new(path: String) -> KindaDb {
        let db_path = format!("{}/db.txt", path);
        let state = match path::Path::new(&db_path).exists() {
            false => HashMap::default(),
            true => {
                let file = fs::read_to_string(&db_path).await.unwrap();

                let raw: Vec<_> = file
                    .split('\n')
                    .filter(|&s| !s.is_empty())
                    .collect();

                let mut acc_map = HashMap::default();

                for record in raw {
                    log::info!("record is {:?}", record);
                    let mut parts = record.split_whitespace();
                    let chat_id = ChatId(parts.next().unwrap().parse::<i64>().unwrap());
                    let last_access = parts.next().unwrap().parse::<u64>().unwrap();

                    let chat_state = match last_access {
                        la if la == 0 => ChatState::Unconfirmed,
                        la => {
                            let chat_path = format!("{}/{}.txt", path, chat_id);

                            let chat_state = match path::Path::new(&chat_path).exists() {
                                true => {
                                    let file = fs::read_to_string(&chat_path)
                                        .await
                                        .unwrap();
                                    let chat_state_vec: Vec<_> = file
                                        .split("***\n")
                                        .filter(|&s| !s.is_empty())
                                        .collect();
                                    chat_state_vec.chunks(2).map(|ch| {
                                        let role: Role = serde_json::from_str(ch[0]).unwrap();
                                        let msg = ch[1].to_string();
                                        ChatCompletionRequestMessage {
                                            role,
                                            content: Some(msg),
                                            name: None,
                                            function_call: None,
                                        }
                                    }).collect()
                                }
                                false => vec![]
                            };

                            ChatState::Confirmed(la, chat_state)
                        }
                    };

                    acc_map.insert(chat_id, chat_state);
                }

                acc_map
            }
        };

        KindaDb {
            path,
            state: Arc::new(RwLock::new(state)),
        }
    }

    async fn save_state(&self, state: &HashMap<ChatId, ChatState>) {
        let db_path = format!("{}/db.txt", self.path);
        let state_str = state.iter().fold(
            String::new(),
            |mut acc, (chat_id, state)| {
                match state {
                    ChatState::Unconfirmed => acc.push_str(&format!("{} 0\n", chat_id)),
                    ChatState::Confirmed(la, _) => acc.push_str(&format!("{} {}\n", chat_id, la)),
                }
                acc
            },
        );

        fs::write(db_path, state_str).await.unwrap()
    }
}
