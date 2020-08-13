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
use tokio::sync::mpsc;
use warp::Filter;
use reqwest::StatusCode;
use once_cell::sync::{OnceCell};
use tokio_postgres::{NoTls};
use arraylib::iter::IteratorExt;


// Клиент БД
pub static DB: OnceCell<tokio_postgres::Client> = OnceCell::new();

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
                              db_register(user_id, chat_name).await;
                              String::from("Регистрация успешна. Если бот не сможет отправить сообщение в чат или его услугами не будут пользоваться более 3-х месяцев, информация о нём будет стёрта, но вы всегда сможете зарегистрировать его заново")
                           }
                           Err(e) => format!("Не удалось отправить сообщение в чат, возможно вы забыли меня в него добавить: {}", e)
                        }
                     }
                  };
                  cx.answer_str(res).await
               }
               Command::Unregister => {
                  let user_id = cx.update.from().unwrap().id;
         
                  // Проверим, что какой-нибудь чат был зарегистрирован
                  let res = match db_user_chat_name(user_id).await {
                     Some(chat_name) => {
                        // Удаляем чат и сообщаем об этом
                        db_unregister(user_id).await;
                        format!("Информация о чате {} удалена", chat_name)
                     }
                     None => String::from("Зарегистрированного вами чата не числится, если вы его регистрировали, то возможно он был удалён автоматически при ошибке отправки в него сообщений или из-за долгого бездействия")
                  };
                  cx.answer_str(res).await
               }
            }
         } else {
            cx.reply_to("Выберите чат для отправки")
            .reply_markup(chats_markup().await)
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
   match DB.set(client) {
      Ok(_) => log::info!("Database connected"),
      _ => log::info!("Something wrong with database"),
   }

   // Создадим таблицу в БД, если её ещё нет
   check_database().await;

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

/// Создаёт таблицу, если её ещё не существует
async fn check_database() {
   // Получаем клиента БД
   let client = DB.get().unwrap();

   // Выполняем запрос
   let rows = client.query("SELECT table_name FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_NAME='chats'", &[]).await.unwrap();

   // Если таблица не существует, создадим её
   if rows.is_empty() {
      client.execute("CREATE TABLE chats (
         PRIMARY KEY (user_id),
         user_id        INTEGER        NOT NULL,
         chat_name      VARCHAR(100)   NOT NULL,
         last_use       TIMESTAMP      NOT NULL
      )", &[]).await.unwrap();
   }
}

/// Регистрация чата для пользователя
async fn db_register(user_id: i32, chat_name: String) {
   let client = DB.get().unwrap();

   // Удалим прежнюю информацию, если пользователь уже регистрировал чат
   db_unregister(user_id).await;

   // Добавляем новую запись, при ошибке сообщение в лог
   if let Err(e) = client.execute("INSERT INTO chats (user_id, chat_name, last_use) VALUES ($1::INTEGER, $2::VARCHAR(100), NOW())", &[&user_id, &chat_name]).await {
      log::error!("db_register({}, {}): {}", user_id, chat_name, e);
   }
}

/// Удаление инормации о пользователе
async fn db_unregister(user_id: i32) {
   let client = DB.get().unwrap();
   
   // Выполняем запрос для удаления записи, при ошибке сообщение в лог
   if let Err(e) = client.execute("DELETE FROM chats WHERE user_id = $1::INTEGER", &[&user_id]).await {
      log::error!("db_unregister({}): {}", user_id, e);
   }
}

/// Возвращает название чата для указанного пользователя
async fn db_user_chat_name(user_id: i32) -> Option<String> {
   let client = DB.get().unwrap();
   let res = client.query_one("SELECT chat_name FROM chats WHERE user_id = $1::INTEGER", &[&user_id]).await;
   match res {
      Ok(data) => Some(data.get(0)),
      _ => None,
   }
}

/// Возвращает идентификатор админа чата
async fn db_user_id(chat_name: &String) -> Option<i32> {
   let client = DB.get().unwrap();
   let res = client.query_one("SELECT user_id FROM chats WHERE chat_name = $1::VARCHAR(100)", &[&chat_name]).await;
   match res {
      Ok(data) => Some(data.get(0)),
      _ => None,
   }
}

/// Возвращает список кнопок с чатами
async fn chats_markup() -> InlineKeyboardMarkup {
   let client = DB.get().unwrap();
   match client.query("SELECT chat_name FROM chats", &[]).await {
      Ok(rows) => {
         // Создадим кнопки
         let mut buttons: Vec<InlineKeyboardButton> = rows.into_iter()
         .map(|row| (InlineKeyboardButton::callback(row.get(0), row.get(0)))).collect();

         // Последняя непарная кнопка, если есть
         let last = if buttons.len() % 2 == 1 { buttons.pop() } else { None };

               // Поделим по две в ряд
         let markup = buttons.into_iter().array_chunks::<[_; 2]>()
         .fold(InlineKeyboardMarkup::default(), |acc, [left, right]| acc.append_row(vec![left, right]));

         // Добавляем последнюю непарную кнопку, если есть, а затем возвращаем результат
         if let Some(last_button) = last {
            markup.append_row(vec![last_button])
         } else {
            markup
         }

      },
      _ => InlineKeyboardMarkup::default(),
   }
}

async fn handle_callback(cx: UpdateWithCx<CallbackQuery>) {
   let query = &cx.update;
   let query_id = &query.id;

   // Сообщение для отправки обратно
   let msg = match &query.data {
      None => {
         String::from("Error No data")
      }
      Some(data) => {
         // Получим текст сообщения
         if let Some(message) = query.message.as_ref().and_then(|s| Message::text(&s)) {
            // Код пользователя
            let user_id = query.from.id;

            // Код администратора по имени чата
            let admin = db_user_id(data).await;

            match admin {
               Some(id) => {

                  // Отправим сообщение администратору на модерацию
                  let chat_id = ChatId::Id(i64::from(id));
                  let res = cx.bot
                  .send_message(chat_id, format!("Поступило сообщение\n{}", message))
                  .send()
                  .await;
                  match res {
                     Ok(_) => {
                        // Ссылка на исправляемое сообщение
                        let original_message = ChatOrInlineMessage::Chat {
                           chat_id: ChatId::Id(i64::from(user_id)),
                           message_id: query.message.as_ref().unwrap().id,
                        };

                        // Отредактируем сообщение у пользователя
                        let res = cx.bot
                        .edit_message_text(original_message, String::from("Сообщение находится на рассмотрении администратора чата и после его одобрения оно появится в чате"))
                        .send().
                        await;
                        
                        match res {
                           Ok(_) => String::from("Успешно"),
                           Err(e) => format!("Ошибка  {}", e),
                        }
                     }
                     Err(e) => format!("Ошибка: {}", e)
                  }
               },
               None => String::from("Error No admin")
            }
         } else {
            String::from("Слишком старое сообщение")
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
}
