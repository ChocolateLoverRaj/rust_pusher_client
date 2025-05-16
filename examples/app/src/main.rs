use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

use dotenvy_macro::dotenv;
use fluvio_wasm_timer::Delay;
use iced::Task;
use iced::futures::lock::Mutex;
use iced::futures::stream::{once, unfold};
use iced::widget::{button, column, text};
use iced::{Element, Subscription, Theme};
use pusher_client::{ConnectionState, Options, PusherClientConnection};

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
    ConnectionStateChange,
    Disconnect,
    Connect,
}

impl App {
    fn update(&mut self, message: Message) -> Task<Message> {
        println!("Message: {:?}", message);
        match message {
            Message::Initial => {
                let (connection_future, connection) = PusherClientConnection::new(Options {
                    cluster_name: dotenv!("PUSHER_CLUSTER_NAME").into(),
                    key: dotenv!("PUSHER_KEY").into(),
                    activity_timeout: Duration::from_secs(1),
                    pong_timeout: Duration::from_secs(5),
                });
                connection.connect();
                let task = Task::batch([
                    Task::run(once(connection_future), |_| Message::Unreachable),
                    Task::run(
                        unfold((), {
                            let receiver = Arc::new(Mutex::new(connection.state().clone()));
                            move |()| {
                                let receiver = receiver.clone();
                                async move { Some(((), receiver.lock().await.changed().await.unwrap())) }
                            }
                        }),
                        |_| Message::ConnectionStateChange,
                    ),
                ]);
                self.connection = Some(connection);
                task
            }
            Message::Unreachable => {
                unreachable!()
            }
            Message::ConnectionStateChange => {
                let connection = self.connection.as_ref().unwrap();
                if let ConnectionState::NotConnected(not_connected_state) =
                    connection.state().borrow().deref()
                {
                    if let Some(_error) = &not_connected_state.error {
                        // Do not use up too much CPU from constantly failing
                        Task::run(once(Delay::new(Duration::from_secs(1))), |_| {
                            Message::Connect
                        })
                    } else {
                        Task::none()
                    }
                } else {
                    Task::none()
                }
            }
            Message::Connect => {
                self.connection.as_ref().unwrap().connect();
                Task::none()
            }
            Message::Disconnect => {
                self.connection.as_ref().unwrap().disconnect();
                Task::none()
            }
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::run(|| once(async { Message::Initial }))
    }

    fn view(&self) -> Element<Message> {
        let mut elements = Vec::from_iter([text(format!(
            "Connection status: {:#?}",
            self.connection
                .as_ref()
                .map(|connection| connection.state().borrow())
                .as_deref()
        ))
        .into()]);
        if let Some(connection) = self.connection.as_ref() {
            match connection.state().borrow().deref() {
                ConnectionState::Connected(_) => {
                    elements.push(
                        button("Disconnect")
                            .style(button::danger)
                            .on_press(Message::Disconnect)
                            .into(),
                    );
                }
                ConnectionState::Disconnecting => {
                    elements.push(button("Disconnecting").style(button::danger).into());
                }
                ConnectionState::NotConnected(_) => {
                    elements.push(
                        button("Connect")
                            .style(button::primary)
                            .on_press(Message::Connect)
                            .into(),
                    );
                }
                ConnectionState::Connecting => {}
            }
        }
        column(elements).into()
    }

    fn theme(&self) -> Theme {
        Theme::Dark
    }
}
