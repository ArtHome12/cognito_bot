/* ===============================================================================
Бот для анонимизации сообщений для чата.
Главный модуль. 12 July 2020.
----------------------------------------------------------------------------
Licensed under the terms of the GPL version 3.
http://www.gnu.org/licenses/gpl-3.0.html
Copyright (c) 2020 by Artem Khomenko _mag12@yahoo.com.
=============================================================================== */

use teloxide::{
   prelude::*,
   utils::command::BotCommand,
   types::{ChatId, InlineKeyboardMarkup, InlineKeyboardButton, CallbackQuery,},
   requests::ResponseResult,
   dispatching::{update_listeners::{self, StatefulListener}, stop_token::AsyncStopToken}
};
use std::{convert::Infallible, env, net::SocketAddr, };
use tokio::{sync::mpsc, time::{sleep, Duration}};
use tokio_stream::wrappers::UnboundedReceiverStream;
use warp::Filter;
use reqwest::{StatusCode, Url};
use rand::Rng;
use native_tls::{TlsConnector};
use postgres_native_tls::MakeTlsConnector;

mod database;
use database as db;

#[derive(BotCommand)]
#[command(rename = "lowercase", description = "Поддерживаются команды:")]
enum Command {
   Start,
   #[command(description = "выводит этот текст.")]
   Help,
   #[command(description = "регистрация новой публичной группы, например для группы t.me/your_chat надо отправить '/register @your_chat', бот должен быть добавлен в этот чат, иначе он не сможет отправлять сообщения. Вы можете быть администратором только одного чата, при регистрации нового предыдущий будет забыт.")]
   Register(String),
   #[command(description = "указание боту забыть чат.")]
   Unregister,
}

async fn handle_message(cx: UpdateWithCx<AutoSend<Bot>, Message>) -> ResponseResult<Message> {

   // Для различения, в личку или в группу пишут
   let chat_id = cx.update.chat_id();

   // Обрабатываем сообщение, только если оно пришло в личку
   if chat_id < 0 {
      return Ok(cx.update);
   }
   
   match cx.update.text() {
      None => cx.answer("Текстовое сообщение, пожалуйста!").await,
      Some(text) => {
         // Попробуем получить команду
         if let Ok(command) = Command::parse(text, "cognito_bot") {
            match command {
               Command::Start => cx.answer(String::from("Добро пожаловать. Отправьте сообщение, выберите чат из списка зарегистрированных в боте и оно будет направлено на модерацию администратору чата (он не будет знать, от кого). Если администратор одобрит его публикацию, сообщение будет отправлено ботом в чат также анонимно. Все поддерживаемые команды: /help")).await,
               Command::Help => cx.answer(Command::descriptions()).await,
               Command::Register(chat_name) => {
                  let res = if chat_name.is_empty() {String::from("После команды /register надо указать имя чата, например если имя вашего чата @your_chat, то введите вручную и отправьте отдельным сообщением /register @your_chat")}
                  else {
                     if chat_name.get(0..1).unwrap_or_default() != "@" {format!("Имя чата должно начинаться со знака @, а вы ввели '{}'", chat_name)}
                     else {
                        // Если чат с таким именем уже зарегистрирован, сообщим об ошибке
                        if db::user_id(&chat_name.clone()).await.is_some() {
                           String::from("Такой чат уже зарегистрирован")
                        } else {
                           // Пробуем отправить приветственное сообщение в чат
                           let chat_id = ChatId::ChannelUsername(chat_name.clone());
                           let res = cx.requester
                           .send_message(chat_id, "Приветствую вас. Я бот-анонимайзер, напишите мне в личку, я от своего имени перешлю сообщение админу и если он одобрит, я от своего имени перешлю его сюда и никто, кроме вас самого, не будет знать, от кого оно")
                           .send()
                           .await;
                           match res {
                              Ok(_) => {
                                 // Всё хорошо, сохраним регистрацию
                                 let user_id = cx.update.from().unwrap().id;
                                 db::register(user_id, chat_name).await;
                                 String::from("Регистрация успешна. Если бот не сможет отправить сообщение в чат или его услугами не будут пользоваться более 3-х месяцев, информация о нём будет стёрта, но вы всегда сможете зарегистрировать его заново")
                              }
                              Err(e) => format!("Не удалось отправить сообщение в чат, возможно вы забыли меня в него добавить: {}", e)
                           }
                        }
                     }
                  };
                  cx.answer(res).await
               }
               Command::Unregister => {
                  let user_id = cx.update.from().unwrap().id;

                  // Проверим, что какой-нибудь чат был зарегистрирован
                  let res = match db::user_chat_name(user_id).await {
                     Some(chat_name) => {
                        // Удаляем чат и сообщаем об этом
                        db::unregister(user_id).await;
                        format!("Информация о чате {} удалена", chat_name)
                     }
                     None => String::from("Зарегистрированного вами чата не числится, если вы его регистрировали, то возможно он был удалён автоматически при ошибке отправки в него сообщений или из-за долгого бездействия")
                  };
                  cx.answer(res).await
               }
            }
         } else {
            cx.reply_to("Выберите чат для отправки")
            .reply_markup(db::chats_markup().await)
            .send()
            .await
         }
      }
   }
}

#[tokio::main]
async fn main() {
   run().await;
}

async fn handle_rejection(error: warp::Rejection) -> Result<impl warp::Reply, Infallible> {
   log::error!("Cannot process the request due to: {:?}", error);
   Ok(StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn webhook<'a>(bot: AutoSend<Bot>) -> impl update_listeners::UpdateListener<Infallible> {
   // Heroku defines auto defines a port value
   let teloxide_token = env::var("TELOXIDE_TOKEN").expect("TELOXIDE_TOKEN env variable missing");
   let port: u16 = env::var("PORT")
      .expect("PORT env variable missing")
      .parse()
      .expect("PORT value to be integer");
   // Heroku host example .: "heroku-ping-pong-bot.herokuapp.com"
   let host = env::var("HOST").expect("have HOST env variable");
   let path = format!("bot{}", teloxide_token);
   let url =  Url::parse(&format!("https://{}/{}", host, path))
   .unwrap();

   bot.set_webhook(url)
      .send()
      .await
      .expect("Cannot setup a webhook");

   let (tx, rx) = mpsc::unbounded_channel();

   let server = warp::post()
      .and(warp::path(path))
      .and(warp::body::json())
      .map(move |json: serde_json::Value| {
         let try_parse = match serde_json::from_str(&json.to_string()) {
               Ok(update) => Ok(update),
               Err(error) => {
                  log::error!(
                     "Cannot parse an update.\nError: {:?}\nValue: {}\n\
                     This is a bug in teloxide, please open an issue here: \
                     https://github.com/teloxide/teloxide/issues.",
                     error,
                     json
                  );
                  Err(error)
               }
         };
         if let Ok(update) = try_parse {
               tx.send(Ok(update))
                  .expect("Cannot send an incoming update from the webhook")
         }

         StatusCode::OK
      })
      .recover(handle_rejection);

   let (stop_token, stop_flag) = AsyncStopToken::new_pair();

   let addr = format!("0.0.0.0:{}", port).parse::<SocketAddr>().unwrap();
   let server = warp::serve(server);
   let (_addr, fut) = server.bind_with_graceful_shutdown(addr, stop_flag);

   // You might want to use serve.key_path/serve.cert_path methods here to
   // setup a self-signed TLS certificate.

   tokio::spawn(fut);
   let stream = UnboundedReceiverStream::new(rx);

   fn streamf<S, T>(state: &mut (S, T)) -> &mut S { &mut state.0 }
   
   StatefulListener::new((stream, stop_token), streamf, |state: &mut (_, AsyncStopToken)| state.1.clone())
}

async fn run() {
   teloxide::enable_logging!();
   log::info!("Starting cognito_bot...");

   let bot = Bot::from_env().auto_send();

   // Логин к БД
   let database_url = env::var("DATABASE_URL").expect("DATABASE_URL env variable missing");
   log::info!("{}", database_url);

   let connector = TlsConnector::builder()
   // .add_root_certificate(cert)
   .danger_accept_invalid_certs(true)
   .build().unwrap();
   let connector = MakeTlsConnector::new(connector);

   // Откроем БД
   let (client, connection) =
      tokio_postgres::connect(&database_url, connector).await
         .expect("Cannot connect to database");

   // The connection object performs the actual communication with the database,
   // so spawn it off to run on its own.
   tokio::spawn(async move {
      if let Err(e) = connection.await {
         log::info!("Database connection error: {}", e);
      }
   });

   // Сохраним доступ к БД
   match db::DB.set(client) {
      Ok(_) => log::info!("Database connected"),
      _ => log::info!("Something wrong with database"),
   }

   // Создадим таблицу в БД, если её ещё нет
   db::check_database().await;

   Dispatcher::new(bot.clone())
   .messages_handler(handle_message_query)
   .callback_queries_handler(handle_callback_query)
   .dispatch_with_listener(
      webhook(bot).await,
      LoggingErrorHandler::with_custom_text("An error from the update listener"),
   )
   .await;
}

async fn handle_callback_query(rx: DispatcherHandlerRx<AutoSend<Bot>, CallbackQuery>) {
   UnboundedReceiverStream::new(rx)
   .for_each_concurrent(None, |cx| async move {
      handle_callback(cx).await
    })
   .await;
}

 async fn handle_message_query(rx: DispatcherHandlerRx<AutoSend<Bot>, Message>) {
   UnboundedReceiverStream::new(rx)
   .for_each_concurrent(None, |cx| async move {
      handle_message(cx).await.expect("Something wrong with the bot!");
   })
   .await;
}

// Возвращает кнопки для администратора
fn admin_markup() -> InlineKeyboardMarkup {
   InlineKeyboardMarkup::default()
   .append_row(vec![InlineKeyboardButton::callback(String::from("🗸 Одобрить"), String::from("+")),
      InlineKeyboardButton::callback(String::from("🗴 Отклонить"), String::from("-")),
   ])
}

// Для хранения отложенного сообщения администратору чата
struct MsgToAdmin {
   pub id: i64,
   pub message: String,
   pub delay: u32,
}

async fn handle_callback(cx: UpdateWithCx<AutoSend<Bot>, CallbackQuery>) {
   let query = &cx.update;
   let query_id = &query.id;

   // Код пользователя
   let user_id = query.from.id;

   // Ссылка сообщение для будущей правки
   let message_id = query.message.as_ref().unwrap().id;

   // Отложенное сообщение администратору чата
   let mut msg_to_admin: Option<MsgToAdmin> = None;

   // Сообщение для отправки обратно
   let msg = match &query.data {
      None => {
         String::from("Error No data")
      }
      Some(data) => {
         // Если в сообщении с кнопкой было процитированное сообщение, получим его
         if let Some(message) = query.message.as_ref()
         .and_then(|s| Message::reply_to_message(&s))
         .and_then(|s| Message::text(&s)) {
            // Код администратора по имени чата
            let admin = db::user_id(data).await;

            match admin {
               Some(id) => {

                  // Время задержки
                  let delay = rand::thread_rng().gen_range(3..723);

                  // Приготовим для отправки сообщение администратору на модерацию
                  msg_to_admin = Some(MsgToAdmin{
                     id,
                     message: String::from(message),
                     delay,
                  });

                  // Отредактируем сообщение у пользователя
                  let res = cx.requester
                  .edit_message_text(user_id, message_id, format!("Сообщение через {} сек. (для маскировки онлайн-активности) будет направлено на рассмотрении администратору чата и после его одобрения оно появится в чате", delay))
                  .send().
                  await;

                  match res {
                     Ok(_) => String::from("Успешно"),
                     Err(e) => format!("Ошибка  {}", e),
                  }
               },
               None => String::from("Error No admin")
            }
         } else {
            // Возможно это было сообщение от админа
            match data.as_str() {
               "+" => {
                  // Отправим сообщение в чат
                  if let Some(message) = query.message.as_ref()
                  .and_then(|s| Message::text(&s)) {
                     // Отредактируем сообщение у администратора, ошибку игнорируем
                     let _= cx.requester
                     .edit_message_text(user_id, message_id, format!("Одобрено:\n{}", message))
                     .send()
                     .await;

                     // Получим имя чата по коду пользователя
                     let chat_name = db::user_chat_name(user_id).await.unwrap_or_default();

                     // Код чата
                     let chat_id = ChatId::ChannelUsername(chat_name);
                     
                     // Отправляем сообщение
                     let res = cx.requester
                     .send_message(chat_id, message)
                     .send()
                     .await;

                     match res {
                        Ok(_) => {
                           db::successful_sent(user_id).await;
                           String::from("Одобрено")
                        },
                        Err(e) => {
                           db::error_happened(user_id).await;
                           format!("Ошибка {}", e)
                        },
                     }
                  } else {String::from("Ошибка, нет сообщения")}
               },
               "-" => {
                  // Отредактируем сообщение у администратора, ошибку игнорируем
                  if let Some(message) = query.message.as_ref()
                  .and_then(|s| Message::text(&s)) {
                     let _= cx.requester
                     .edit_message_text(user_id, message_id, format!("Отклонено:\n{}", message))
                     .send()
                     .await;
                  }
                  String::from("Отклонено")
               },
               _ => String::from("Слишком старое сообщение"),
            }
         }
      }
   };

   // Отправляем ответ, который показывается во всплывающем окошке
   match cx.requester.answer_callback_query(query_id)
      .text(&msg)
      .send()
      .await {
         Err(_) => log::info!("Error handle_message {}", &msg),
         _ => (),
   }

   // Если приготовлено отложенное сообщение, надо отправить его
   if let Some(msg) = msg_to_admin {
      // Выжидаем паузу
      sleep(Duration::from_secs(u64::from(msg.delay))).await;

      // Отправляем сообщение админу
      let res = cx.requester
      .send_message(ChatId::Id(msg.id), msg.message)
      .reply_markup(admin_markup())
      .send()
      .await;

      // Фиксируем ошибку, если была, при этом не фиксируем успешную отправку, чтобы не обнулить счётчик отправок в чат
      if res.is_err() {
         db::error_happened(msg.id).await;
      }
   }
}
