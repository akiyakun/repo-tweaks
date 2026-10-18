//! repo-tweaks CLI (Step 0: talk to the server from a git hook)

mod client;
mod config;
mod git;

use std::process::ExitCode;

use clap::{Parser, Subcommand};

use client::{Client, Event};

#[derive(Parser)]
#[command(name = "repo-tweaks", version, about = "repo-tweaks の CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// サーバーにつながるか（トークンが正しいか）を確かめる
    Ping,
    /// 今のリポジトリの情報をサーバーへ送る（git hook から呼ぶ）
    Notify {
        /// 通知の種類（例: post-commit）
        #[arg(long, default_value = "manual")]
        event: String,
    },
    /// サーバーが受け取った通知の直近の一覧を表示する
    Events {
        /// 表示する件数
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli.command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("repo-tweaks: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(command: Command) -> Result<(), String> {
    let git_config = git::GitConfig::read();
    let client = Client::new(config::Config::load(&git_config)?);
    match command {
        Command::Ping => {
            let version = client.ping()?;
            println!("OK: {} につながりました（サーバー {version}）", client.server());
        }
        Command::Notify { event } => {
            let info = git::RepoInfo::current(&git_config);
            let id = client.send_event(&Event::new(event, info))?;
            println!("送信しました（#{id}）");
        }
        Command::Events { limit } => {
            let events = client.events()?;
            if events.is_empty() {
                println!("まだ通知はありません");
            }
            for e in events.iter().take(limit) {
                println!("{e}");
            }
        }
    }
    Ok(())
}
