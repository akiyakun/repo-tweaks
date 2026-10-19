//! repo-tweaks CLI

mod check;
mod client;
mod config;
mod install;
mod list;
mod lock;
mod status;
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
        /// サーバー（Discord）に通知しない
        #[arg(long)]
        no_notify: bool,
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
        /// サーバー（Discord）に通知しない
        #[arg(long)]
        no_notify: bool,
    },
    /// ロックが必要なファイル（.gitattributes の lockable）の、ロックと変更の状態を表示する
    Status {
        /// ロック中・変更中でないものも含めて、すべて表示する
        #[arg(long)]
        all: bool,
        /// JSON で出力する（すべてのファイル。ほかのツールから使うとき）
        #[arg(long)]
        json: bool,
    },
    /// コミットしようとしている変更を確かめる（pre-commit hook から呼ぶ）。ロックしていない lockable のファイルがあれば止める
    Check,
    /// このリポジトリに pre-commit と post-commit の hook を入れる
    InstallHooks {
        /// 別の hook があっても、.bak に残して上書きする
        #[arg(long)]
        force: bool,
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
        Ok(code) => code,
        Err(message) => {
            eprintln!("repo-tweaks: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(command: Command) -> Result<ExitCode, String> {
    let git_config = git::GitConfig::read();
    // Only commands that talk to the repo-tweaks server need its address and token.
    let client = || config::Config::load(&git_config).map(Client::new);
    match command {
        Command::List { json, stale_days } => list::run(&git_config, json, stale_days)?,
        Command::Lock { paths, no_meta, json, no_notify } => {
            lock::lock(&git_config, &paths, &lock::Options { with_meta: !no_meta, json, no_notify })?
        }
        Command::Unlock { paths, force, yes, no_meta, json, no_notify } => {
            lock::unlock(&git_config, &paths, &lock::Options { with_meta: !no_meta, json, no_notify }, force, yes)?
        }
        Command::Status { all, json } => status::run(&git_config, all, json)?,
        // Decides its own exit status: only a blocked commit fails
        Command::Check => return Ok(check::run(&git_config)),
        Command::InstallHooks { force } => install::run(force)?,
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
    Ok(ExitCode::SUCCESS)
}
