//! repo-tweaks CLI

mod client;
mod config;
mod list;
mod lock;
mod text;

use std::process::ExitCode;

use clap::{Parser, Subcommand};

use client::{Client, Event};
use repo_tweaks_core::git;

#[derive(Parser)]
#[command(name = "repo-tweaks", version, about = "repo-tweaks の CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// ロックの一覧を表示する（自分のロック / 他人のロック、古いロックに印）
    List {
        /// JSON で出力する（ほかのツールから使うとき）
        #[arg(long)]
        json: bool,
        /// 何日以上で「古い」とするか（既定は git config repo-tweaks.staleDays、無ければ 3）
        #[arg(long)]
        stale_days: Option<u32>,
    },
    /// ファイルをロックする（Unity の .meta があれば一緒にロック）
    Lock {
        /// ロックするファイル（複数可）
        #[arg(required = true)]
        paths: Vec<String>,
        /// .meta を自動で追加しない
        #[arg(long)]
        no_meta: bool,
        /// 結果を JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// ロックを解除する（ロック中の .meta も一緒に解除）
    Unlock {
        /// 解除するファイル（複数可）
        #[arg(required = true)]
        paths: Vec<String>,
        /// 他人のロックも強制解除する。自分のロックは未コミットの変更があっても解除する
        #[arg(long)]
        force: bool,
        /// --force の確認を飛ばす（ほかのツールから呼ぶとき。確認は呼ぶ側で行う）
        #[arg(long, requires = "force")]
        yes: bool,
        /// .meta を自動で追加しない
        #[arg(long)]
        no_meta: bool,
        /// 結果を JSON で出力する
        #[arg(long)]
        json: bool,
    },
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
    // Only commands that talk to the repo-tweaks server need its address and token.
    let client = || config::Config::load(&git_config).map(Client::new);
    match command {
        Command::List { json, stale_days } => list::run(&git_config, json, stale_days)?,
        Command::Lock { paths, no_meta, json } => lock::lock(&git_config, &paths, !no_meta, json)?,
        Command::Unlock { paths, force, yes, no_meta, json } => {
            lock::unlock(&git_config, &paths, !no_meta, force, yes, json)?
        }
        Command::Ping => {
            let client = client()?;
            let version = client.ping()?;
            println!("OK: {} につながりました（サーバー {version}）", client.server());
        }
        Command::Notify { event } => {
            let client = client()?;
            let info = git::RepoInfo::current(&git_config);
            let id = client.send_event(&Event::new(event, info))?;
            println!("送信しました（#{id}）");
        }
        Command::Events { limit } => {
            let events = client()?.events()?;
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
