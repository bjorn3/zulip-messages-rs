use std::{collections::HashMap, error::Error, fmt, sync::Arc};

use reqwest::Client;

#[derive(Debug, serde::Deserialize)]
struct Config {
    sites: Vec<ZulipSite>,
}

#[derive(Clone, Debug, serde::Deserialize)]
struct ZulipSite {
    name: String,
    user: String,
    token: String,
}

impl ZulipSite {
    fn api_url(&self, api: &str) -> String {
        format!("https://{}.zulipchat.com/api/v1/{}", self.name, api)
    }

    fn get(&self, client: &Client, api: &str) -> reqwest::RequestBuilder {
        client
            .get(&self.api_url(api))
            .basic_auth(&self.user, Some(&self.token))
    }

    fn post(&self, client: &Client, api: &str) -> reqwest::RequestBuilder {
        client
            .post(&self.api_url(api))
            .basic_auth(&self.user, Some(&self.token))
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "result")]
enum ApiResult<T> {
    #[serde(rename = "success")]
    Success(T),
    #[serde(rename = "error")]
    Error(HashMap<String, serde_json::Value>),
}

impl<T> ApiResult<T> {
    fn into_result(self) -> Result<T, Box<dyn std::error::Error + Send + Sync>> {
        match self {
            ApiResult::Success(val) => Ok(val),
            ApiResult::Error(err) => Err(format!("api call failed: {:?}", err).into()),
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let config: Config = serde_json::de::from_str(&std::fs::read_to_string("config.json")?)?;

    let res = futures::future::try_join_all(config.sites.into_iter().map(|site| {
        tokio::spawn(async move {
            let site = Arc::new(site);
            println!("watching {}", site.name);

            let client = Client::builder()
                .user_agent("zulip client by @bjorn3")
                .build()?;

            let mut queue = EventQueue::register_new(&client, site.clone()).await?;

            println!("queue for {}: {}", site.name, queue.queue_id.0);

            loop {
                let events = queue.long_poll(&client).await?;
                for event in events {
                    match event.rest {
                        EventType::Heartbeat => {}
                        EventType::Message { flags, message } => {
                            let is_important = flags.contains(&MessageFlag::Mentioned)
                                || flags.contains(&MessageFlag::HasAlertWord);

                            println!(
                                "{} {:<20} {}",
                                if is_important { "!" } else { " " },
                                site.name,
                                message
                            );

                            if is_important {
                                notify_rust::Notification::new()
                                    .summary(&format!("{} {}", site.name, message.header()))
                                    .body(&message.content)
                                    .show()?;
                            }
                        }
                        EventType::Other => println!("unknown event"),
                    }
                }
            }

            Ok::<(), Box<dyn Error + Send + Sync>>(())
        })
    }))
    .await;

    println!("{:?}", res);

    Ok(())
}

#[derive(Debug, serde::Deserialize)]
#[serde(transparent)]
struct EventQueueId(String);

#[derive(Debug)]
struct EventQueue {
    site: Arc<ZulipSite>,
    queue_id: EventQueueId,
    last_event_id: i64,
}

#[derive(Debug, serde::Deserialize)]
struct RegisterEventQueue {
    last_event_id: i64,
    queue_id: EventQueueId,
}

#[derive(Debug, serde::Deserialize)]
struct PollEventQueue {
    events: Vec<Event>,
}

#[derive(Debug, serde::Deserialize)]
struct Event {
    id: i64,
    #[serde(flatten)]
    rest: EventType,
}

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
enum EventType {
    Heartbeat,

    Message {
        flags: Vec<MessageFlag>,
        message: Message,
    },

    #[serde(other)]
    Other,
}

#[derive(Debug, serde::Deserialize)]
struct Message {
    content: String,
    display_recipient: MessageRecipients,
    //id: u64,
    //reactions: Vec<serde_json::Value>,
    sender_full_name: String,
    //stream_id: Option<u64>,
    #[serde(deserialize_with = "chrono::serde::ts_seconds::deserialize")]
    timestamp: chrono::DateTime<chrono::Utc>,
    #[serde(rename = "type")]
    type_: String, // stream or private
}

impl Message {
    fn header(&self) -> String {
        format!(
            "[{}] @{} -> {}",
            self.timestamp.with_timezone(&chrono::Local).time(),
            self.sender_full_name,
            self.display_recipient,
        )
    }
}

impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.header(), self.content)
    }
}

#[derive(Debug, Eq, PartialEq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum MessageFlag {
    Read,
    Mentioned,
    HasAlertWord,
}

#[derive(Debug, serde::Deserialize)]
#[serde(untagged)]
enum MessageRecipients {
    Stream(String),
    Users(Vec<User>),
}

impl fmt::Display for MessageRecipients {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MessageRecipients::Stream(stream) => write!(f, "#{}", stream),
            MessageRecipients::Users(users) => {
                let mut users = users.iter();
                if let Some(first_user) = users.next() {
                    write!(f, "{}", first_user)?;
                } else {
                    write!(f, "<no users>")?;
                }
                for user in users {
                    write!(f, ",{}", user)?;
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct User {
    full_name: String,
}

impl fmt::Display for User {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "@{}", self.full_name)
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum MessageType {
    Stream,
    Private,
}

impl EventQueue {
    async fn register_new(
        client: &Client,
        site: Arc<ZulipSite>,
    ) -> Result<EventQueue, Box<dyn Error + Send + Sync>> {
        let register_resp = site
            .post(
                client,
                "register?event_types=%5B%22message%22%5D&all_public_streams=false",
            )
            .send()
            .await?
            .json::<ApiResult<RegisterEventQueue>>()
            .await?
            .into_result()?;

        Ok(EventQueue {
            site,
            queue_id: register_resp.queue_id,
            last_event_id: register_resp.last_event_id,
        })
    }

    async fn long_poll(
        &mut self,
        client: &Client,
    ) -> Result<Vec<Event>, Box<dyn std::error::Error + Send + Sync>> {
        let poll_resp = self
            .site
            .get(
                client,
                &format!(
                    "events?queue_id={}&last_event_id={}&dont_block=false",
                    self.queue_id.0, self.last_event_id
                ),
            )
            .send()
            .await?
            .json::<ApiResult<PollEventQueue>>()
            .await?;

        let poll_resp = match poll_resp {
            ApiResult::Error(err)
                if err.get("code").and_then(|code| code.as_str()) == Some("BAD_EVENT_QUEUE_ID") =>
            {
                *self = Self::register_new(client, self.site.clone()).await?;
                return Ok(vec![]);
            }
            poll_resp => poll_resp.into_result()?,
        };

        self.last_event_id = poll_resp
            .events
            .last()
            .expect("either a real event or an heartbeat event should have been returned")
            .id;

        Ok(poll_resp.events)
    }
}
