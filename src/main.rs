/* ===============================================================================
Бот для анонимизации сообщений для чата.
Главный модуль. 12 July 2020.
----------------------------------------------------------------------------
Licensed under the terms of the GPL version 3.
http://www.gnu.org/licenses/gpl-3.0.html
Copyright (c) 2020 by Artem Khomenko _mag12@yahoo.com.
=============================================================================== */

use teloxide::{
   dispatching::update_listeners,
   prelude::*,
   utils::command::BotCommand,
   types::{ChatId, InlineKeyboardMarkup, InlineKeyboardButton, CallbackQuery, 
      ChatOrInlineMessage,
   },
};
use std::{convert::Infallible, env, net::SocketAddr, };
use tokio::{sync::mpsc, time::delay_for,};
use std::time::Duration;
use warp::Filter;
use reqwest::StatusCode;
use tokio_postgres::{NoTls};
use rand::Rng;

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

// async fn handle_message(cx: UpdateWithCx) {
async fn handle_message(cx: UpdateWithCx<Message>) -> ResponseResult<Message> {

   // Для различения, в личку или в группу пишут
   let chat_id = cx.update.chat_id();

   // Обрабатываем сообщение, только если оно пришло в личку
   if chat_id < 0 {
      return Ok(cx.update);
   }
   
   match cx.update.text() {
      None => cx.answer_str("Текстовое сообщение, пожалуйста!").await,
      Some(text) => {
         // Попробуем получить команду
         if let Ok(command) = Command::parse(text, "cognito_bot") {
            match command {
               Command::Start => cx.answer_str(String::from("Добро пожаловать. Отправьте сообщение, выберите чат из списка зарегистрированных в боте и оно будет направлено на модерацию администратору чата (он не будет знать, от кого). Если администратор одобрит его публикацию, сообщение будет отправлено ботом в чат также анонимно. Все поддерживаемые команды: /help")).await,
               Command::Help => cx.answer_str(Command::descriptions()).await,
               Command::Register(chat_name) => {
                  let res = if chat_name.is_empty() {String::from("После команды /register надо указать имя чата, например если имя вашего чата @your_chat, то введите вручную и отправьте отдельным сообщением /register @your_chat")}
                  else {
                     if &chat_name[..1] != "@" {format!("Имя чата должно начинаться со знака @, а вы ввели '{}'", chat_name)}
                     else {
                        // Если чат с таким именем уже зарегистрирован, сообщим об ошибке
                        if db::user_id(&chat_name.clone()).await.is_some() {
                           String::from("Такой чат уже зарегистрирован")
                        } else {
                           // Пробуем отправить приветственное сообщение в чат
                           let chat_id = ChatId::ChannelUsername(chat_name.clone());
                           let res = cx.bot
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
                  cx.answer_str(res).await
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
                  cx.answer_str(res).await
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

pub async fn webhook<'a>(bot: Bot) -> impl update_listeners::UpdateListener<Infallible> {
   // Heroku defines auto defines a port value
   let teloxide_token = env::var("TELOXIDE_TOKEN").expect("TELOXIDE_TOKEN env variable missing");
   let port: u16 = env::var("PORT")
      .expect("PORT env variable missing")
      .parse()
      .expect("PORT value to be integer");
   // Heroku host example .: "heroku-ping-pong-bot.herokuapp.com"
   let host = env::var("HOST").expect("have HOST env variable");
   let path = format!("bot{}", teloxide_token);
   let url = format!("https://{}/{}", host, path);

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

   let serve = warp::serve(server);

   let address = format!("0.0.0.0:{}", port);
   tokio::spawn(serve.run(address.parse::<SocketAddr>().unwrap()));
   rx
}

async fn run() {
   teloxide::enable_logging!();
   log::info!("Starting cognito_bot...");

   let bot = Bot::from_env();

   // Логин к БД
   let database_url = env::var("DATABASE_URL").expect("DATABASE_URL env variable missing");    
   // Откроем БД
   let (client, connection) =
      tokio_postgres::connect(&database_url, NoTls).await
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

   // teloxide::commands_repl_with_listener(bot.clone(), "cognito_bot", answer, webhook(bot).await).await;
   Dispatcher::new(bot.clone())
   .messages_handler(|rx: DispatcherHandlerRx<Message>| {
      rx.for_each_concurrent(None, |message| async move {
         handle_message(message).await.expect("Something wrong with the bot!");
      })
   })
   .callback_queries_handler(|rx: DispatcherHandlerRx<CallbackQuery>| {
      rx.for_each_concurrent(None, |cx| async move {
         handle_callback(cx).await
      })
   })
   .dispatch_with_listener(
      webhook(bot).await,
      LoggingErrorHandler::with_custom_text("An error from the update listener"),
   )
   .await;
}

/// Возвращает кнопки для администратора
fn admin_markup() -> InlineKeyboardMarkup {
   InlineKeyboardMarkup::default()
   .append_row(vec![InlineKeyboardButton::callback(String::from("🗸 Одобрить"), String::from("+")),
      InlineKeyboardButton::callback(String::from("🗴 Отклонить"), String::from("-")),
   ])
}

// Для хранения отложенного сообщения администратору чата
struct MsgToAdmin {
   pub chat_id: ChatId,
   pub message: String,
   pub delay: u32,
}

async fn handle_callback(cx: UpdateWithCx<CallbackQuery>) {
   let query = &cx.update;
   let query_id = &query.id;

   // Код пользователя
   let user_id = query.from.id;

   // Ссылка сообщение для будущей правки
   let original_message = ChatOrInlineMessage::Chat {
      chat_id: ChatId::Id(i64::from(user_id)),
      message_id: query.message.as_ref().unwrap().id,
   };

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
                  let delay = rand::thread_rng().gen_range(3, 723);

                  // Приготовим для отправки сообщение администратору на модерацию
                  msg_to_admin = Some(MsgToAdmin{
                     chat_id: ChatId::Id(i64::from(id)),
                     message: String::from(message),
                     delay,
                  });

                  // Отредактируем сообщение у пользователя
                  let res = cx.bot
                  .edit_message_text(original_message, format!("Сообщение через {} сек. (для маскировки онлайн-активности) будет направлено на рассмотрении администратору чата и после его одобрения оно появится в чате", delay))
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
                     let _= cx.bot
                     .edit_message_text(original_message, format!("Одобрено:\n{}", message))
                     .send()
                     .await;

                     // Получим имя чата по коду пользователя
                     let chat_name = db::user_chat_name(user_id).await.unwrap_or_default();

                     // Код чата
                     let chat_id = ChatId::ChannelUsername(chat_name);
                     
                     // Отправляем сообщение
                     let res = cx.bot
                     .send_message(chat_id, message)
                     .send()
                     .await;

                     match res {
                        Ok(_) => String::from("Одобрено"),
                        Err(e) => format!("Ошибка {}", e),
                     }
                  } else {String::from("Ошибка, нет сообщения")}
               },
               "-" => {
                  // Отредактируем сообщение у администратора, ошибку игнорируем
                  if let Some(message) = query.message.as_ref()
                  .and_then(|s| Message::text(&s)) {
                     let _= cx.bot
                     .edit_message_text(original_message, format!("Отклонено:\n{}", message))
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
   match cx.bot.answer_callback_query(query_id)
      .text(&msg)
      .send()
      .await {
         Err(_) => log::info!("Error handle_message {}", &msg),
         _ => (),
   }

   // Если приготовлено отложенное сообщение, надо отправить его
   if let Some(msg) = msg_to_admin {
      // Выжидаем паузу
      delay_for(Duration::from_secs(u64::from(msg.delay))).await;

      // Отправляем сообщение админу
      let _res = cx.bot
      .send_message(msg.chat_id, msg.message)
      .reply_markup(admin_markup())
      .send()
      .await;
   }
}
