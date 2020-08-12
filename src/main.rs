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
   types::{ChatId,},
};
use std::{convert::Infallible, env, net::SocketAddr};
use tokio::sync::mpsc;
use warp::Filter;
use reqwest::StatusCode;
use once_cell::sync::{OnceCell};
use tokio_postgres::{NoTls};


// Клиент БД
pub static DB: OnceCell<tokio_postgres::Client> = OnceCell::new();

#[derive(BotCommand)]
#[command(rename = "lowercase", description = "Поддерживаются команды:")]
enum Command {
   #[command(description = "отправьте сообщение или зарегистрируйте свой чат.")]
   Start,
   #[command(description = "выводит этот текст.")]
   Help,
   #[command(description = "регистрация нового чата, например '/register @your_chat', бот должен быть добавлен в этот чат, иначе он не сможет отправлять сообщения. Вы можете быть администратором только одного чата, при регистрации нового предыдущий будет забыт.")]
   Register(String),
   #[command(description = "указание боту забыть чат, например '/unregister @your_chat'.")]
   Unregister(String),
}

async fn answer(cx: UpdateWithCx<Message>, command: Command) -> ResponseResult<()> {
    match command {
      Command::Start => cx.answer("Добро пожаловать. Отправьте сообщение, выберите чат из списка зарегистрированных в боте и оно будет направлено на модерацию администратору чата (он не будет знать, от кого). Если администратор одобрит его публикацию, сообщение будет отправлено ботом в чат также анонимно. Все поддерживаемые команды: /help").send().await?,
      Command::Help => cx.answer(Command::descriptions()).send().await?,
      Command::Register(chat_name) => {
         let res = if chat_name.is_empty() {String::from("После команды /start надо указать имя чата, например если имя вашего чата @your_chat, то введите вручную и отправьте отдельным сообщением /register @your_chat")}
         else {
            if &chat_name[..1] != "@" {format!("Имя чата должно начинаться со знака @, а вы ввели '{}'", chat_name)}
            else {
               // Пробуем отправить приветственное сообщение в чат
               let chat_id = ChatId::ChannelUsername(chat_name);
               let res = cx.bot
               .send_message(chat_id, "Приветствую вас. Я бот-анонимайзер, напишите мне в личку, я от своего имени перешлю сообщение админу и если он одобрит, я от своего имени перешлю его сюда и никто, кроме автора, не будет знать, от кого оно")
               .send()
               .await;
               match res {
                  Ok(_) => {
                     // Всё хорошо, сохраним регистрацию
                     String::from("Регистрация успешна")
                  }
                  Err(e) => format!("Не удалось отправить сообщение в чат, возможно вы забыли меня в него добавить: {}", e)
               }
            }
         };
         cx.answer_str(res).await?
      }
      Command::Unregister(chat_name) => {
         cx.answer_str(format!("Your chat_name is @{}.", chat_name)).await?
      }
   };

   Ok(())
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
   let cloned_bot = bot.clone();

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

   teloxide::commands_repl_with_listener(bot, "cognito_bot", answer, webhook(cloned_bot).await).await;
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
         CREATE TABLE users (
            PRIMARY KEY (user_id),
            user_id        INTEGER        NOT NULL,
            user_name      VARCHAR(100)   NOT NULL,
            contact        VARCHAR(100)   NOT NULL,
            address        VARCHAR(100)   NOT NULL,
            last_seen      TIMESTAMP      NOT NULL,
         ", &[]).await.unwrap();
   }
}

