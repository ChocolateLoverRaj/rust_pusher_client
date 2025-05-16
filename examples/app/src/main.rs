use std::time::Duration;

use dotenvy_macro::dotenv;
use iced::Task;
use iced::futures::stream::once;
use iced::widget::text;
use iced::{Element, Subscription, Theme};
use pusher_client::{Options, PusherClientConnection};

pub fn main() -> iced::Result {
    console_error_panic_hook::set_once();
    iced::application("Pusher Client Demo", App::update, App::view)
        .subscription(App::subscription)
        .theme(App::theme)
        .run()
}

#[derive(Default)]
struct App {
    connection: Option<PusherClientConnection>,
}

#[derive(Default)]
enum State {
    #[default]
    Idle,
}

#[derive(Debug, Clone)]
enum Message {
    Initial,
    Unreachable,
}

impl App {
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Initial => {
                let (connection_future, connection) = PusherClientConnection::new(Options {
                    cluster_name: dotenv!("PUSHER_CLUSTER_NAME").into(),
                    key: dotenv!("PUSHER_KEY").into(),
                    activity_timeout: Duration::from_secs(1),
                    pong_timeout: Duration::from_secs(5),
                });
                connection.connect();
                self.connection = Some(connection);
                Task::run(once(connection_future), |_| Message::Unreachable)
            }
            Message::Unreachable => {
                unreachable!()
            }
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::run(|| once(async { Message::Initial }))
    }

    fn view(&self) -> Element<Message> {
        text("Hello").size(100).into()
    }

    fn theme(&self) -> Theme {
        Theme::Dark
    }
}
