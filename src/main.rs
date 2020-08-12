/* ===============================================================================
Бот для анонимизации сообщений для чата.
Главный модуль. 12 July 2020.
----------------------------------------------------------------------------
Licensed under the terms of the GPL version 3.
http://www.gnu.org/licenses/gpl-3.0.html
Copyright (c) 2020 by Artem Khomenko _mag12@yahoo.com.
=============================================================================== */

use teloxide::{prelude::*, utils::command::BotCommand};

#[derive(BotCommand)]
#[command(rename = "lowercase", description = "These commands are supported:")]
enum Command {
    #[command(description = "display this text.")]
    Help,
    #[command(description = "handle a username.")]
    Username(String),
    #[command(description = "handle a username and an age.", parse_with = "split")]
    UsernameAndAge { username: String, age: u8 },
}

async fn answer(cx: UpdateWithCx<Message>, command: Command) -> ResponseResult<()> {
    match command {
        Command::Help => cx.answer(Command::descriptions()).send().await?,
        Command::Username(username) => {
            cx.answer_str(format!("Your username is @{}.", username)).await?
        }
        Command::UsernameAndAge { username, age } => {
            cx.answer_str(format!("Your username is @{} and age is {}.", username, age)).await?
        }
    };

    Ok(())
}

#[tokio::main]
async fn main() {
    run().await;
}

async fn run() {
    teloxide::enable_logging!();
    log::info!("Starting simple_commands_bot...");

    let bot = Bot::from_env();

    let bot_name: String = panic!("Your bot's name here");
    teloxide::commands_repl(bot, bot_name, answer).await;
}