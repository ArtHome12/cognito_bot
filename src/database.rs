/* ===============================================================================
Бот для анонимизации сообщений для чата.
Взаимодействие с базой данных. 14 July 2020.
----------------------------------------------------------------------------
Licensed under the terms of the GPL version 3.
http://www.gnu.org/licenses/gpl-3.0.html
Copyright (c) 2020 by Artem Khomenko _mag12@yahoo.com.
=============================================================================== */

use once_cell::sync::{OnceCell};
use arraylib::iter::IteratorExt;
use teloxide::{
   types::{InlineKeyboardMarkup, InlineKeyboardButton, },
};

// Клиент БД
pub static DB: OnceCell<tokio_postgres::Client> = OnceCell::new();

/// Создаёт таблицу, если её ещё не существует
pub async fn check_database() {
   // Получаем клиента БД
   let client = DB.get().unwrap();

   // Выполняем запрос
   let rows = client.query("SELECT table_name FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_NAME='chats'", &[]).await.unwrap();

   // Если таблица не существует, создадим её
   if rows.is_empty() {
      client.execute("CREATE TABLE chats (
         PRIMARY KEY (user_id),
         user_id        BIGINT         NOT NULL,
         chat_name      VARCHAR(100)   NOT NULL,
         last_use       TIMESTAMP      NOT NULL,
         errors         INTEGER        NOT NULL
      )", &[]).await.unwrap();
   }
}

/// Регистрация чата для пользователя
pub async fn register(user_id: i64, chat_name: String) {
   let client = DB.get().unwrap();

   // Удалим прежнюю информацию, если пользователь уже регистрировал чат
   unregister(user_id).await;

   // Добавляем новую запись, при ошибке сообщение в лог
   if let Err(e) = client.execute("INSERT INTO chats (user_id, chat_name, last_use, errors) VALUES ($1::BIGINT, $2::VARCHAR(100), NOW(), 0)", &[&user_id, &chat_name]).await {
      log::error!("db_register({}, {}): {}", user_id, chat_name, e);
   }
}

/// Удаление инормации о пользователе
pub async fn unregister(user_id: i64) {
   let client = DB.get().unwrap();
   
   // Выполняем запрос для удаления записи, при ошибке сообщение в лог
   if let Err(e) = client.execute("DELETE FROM chats WHERE user_id = $1::BIGINT", &[&user_id]).await {
      log::error!("db_unregister({}): {}", user_id, e);
   }
}

/// Возвращает название чата для указанного пользователя
pub async fn user_chat_name(user_id: i64) -> Option<String> {
   let client = DB.get().unwrap();
   let res = client.query_one("SELECT chat_name FROM chats WHERE user_id = $1::BIGINT", &[&user_id]).await;
   match res {
      Ok(data) => Some(data.get(0)),
      _ => None,
   }
}

/// Возвращает идентификатор админа чата
pub async fn user_id(chat_name: &String) -> Option<i64> {
   let client = DB.get().unwrap();
   let res = client.query_one("SELECT user_id FROM chats WHERE chat_name = $1::VARCHAR(100)", &[&chat_name]).await;
   match res {
      Ok(data) => Some(data.get(0)),
      _ => None,
   }
}

/// Возвращает список кнопок с чатами
pub async fn chats_markup() -> InlineKeyboardMarkup {
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

/// Увеличивает счётчик ошибок и если стало слишком много, удаляет чат
/// Функция должна вызываться при каждой ошибке отправки сообщения админу или в чат
pub async fn error_happened(user_id: i64) {
   let client = DB.get().unwrap();

   // Увеличиваем счётчик ошибок
   if let Err(e) = client.execute("UPDATE chats SET errors = errors + 1 WHERE user_id = $1::BIGINT", &[&user_id]).await {
      log::error!("error_happened({}): {}", user_id, e);
      return;
   };

   // Читаем счётчик ошибок
   let res = client.query_one("SELECT errors FROM chats WHERE user_id = $1::BIGINT", &[&user_id]).await;
   match res {
      Ok(data) => {
         // Если ошибок слишком много, забываем чат
         let cnt: i32 = data.get(0);
         if cnt > 3 {unregister(user_id).await;}
      }
      // При ошибке сообщаем в лог и выходим
      Err(e) => log::error!("error_happened 2 ({}): {}", user_id, e),
   }
}

/// Обнуляет счётчик ошибок отправки сообщений
/// Функция должна вызываеться после каждой успешной попытки записи в чат, но не при
/// успешной отправке сообщения админу, иначе это сбросит более приоритетный счётчик
/// ошибок в чат
pub async fn successful_sent(user_id: i64) {
   let client = DB.get().unwrap();
   if let Err(e) = client.execute("UPDATE chats SET errors = 0 WHERE user_id = $1::BIGINT", &[&user_id]).await {
      log::error!("successful_sent({}): {}", user_id, e);
   };
}