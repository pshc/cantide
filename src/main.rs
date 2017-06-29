#![recursion_limit = "1024"]

#[macro_use]
extern crate error_chain;
extern crate irc;
extern crate postgres;
extern crate rand;

use std::env;
use std::default::Default;
use std::io::{self, Write};
use std::sync::Mutex;

use irc::client::prelude::*;
use postgres::{Connection, TlsMode};

use errors::*;

struct Cantide {
    brain: Mutex<Brain>,
    channel: String,
    irc: IrcServer,
}

impl Cantide {
    pub fn serve(&self) -> Result<()> {
        self.irc.for_each_incoming(|msg| {
            if let Err(e) = self.handle(msg) {
                let stderr = &mut io::stderr();
                let oops = "couldn't write to stderr";

                writeln!(stderr, "error: {}", e).expect(oops);
                for e in e.iter().skip(1) {
                    writeln!(stderr, "  caused by: {}", e).expect(oops);
                }
            }
        }).map_err(|e| e.into())
    }

    pub fn handle(&self, msg: Message) -> Result<()> {
        match msg.command {
            Command::JOIN(ref chan, _, _) if chan == &self.channel => {
                if let Some(nick) = msg.source_nickname() {
                    println!("* {} joined {}", nick, chan);
                }
            }
            Command::NICK(nick) => {
                println!("My nick changed to: {}", nick);
                self.brain.lock().unwrap().nick = nick;
            }
            Command::NOTICE(src, msg) => println!("Notice from {}: {:?}", src, msg),
            Command::PART(ref chan, ref part_msg) if chan == &self.channel => {
                if let Some(nick) = msg.source_nickname() {
                    if let &Some(ref bye) = part_msg {
                        println!("* {} left {} ({})", nick, chan, bye);
                    } else {
                        println!("* {} left {}", nick, chan);
                    }
                }
            }
            Command::PING(_, _) => (),
            Command::PRIVMSG(ref target, ref text) if target == &self.channel => {
                let nick = msg.source_nickname().ok_or_else(|| Error::from("nick missing"))?;
                println!("<{}> {}", nick, text);
                self.respond_to(text)?;
            }
            Command::QUIT(ref quit_msg) => {
                if let Some(nick) = msg.source_nickname() {
                    if let &Some(ref bye) = quit_msg {
                        println!("* {} quit ({})", nick, bye);
                    } else {
                        println!("* {} quit", nick);
                    }
                }
            }
            Command::Response(resp, args, text) => {
                use irc::proto::response::Response::*;
                match resp {
                    RPL_ISUPPORT | RPL_MOTDSTART | RPL_MOTD | RPL_ENDOFMOTD => (),
                    RPL_WELCOME => {
                        if args.len() == 1 {
                            let nick = args[0].clone();
                            println!("My nick: {}", nick);
                            self.brain.lock().unwrap().nick = nick;
                        }
                    }
                    r => {
                        match text {
                            Some(text) => println!("> {:?} {:?} {}", r, args, text),
                            None => println!("> {:?} {:?}", r, args),
                        }
                    }
                }
            }
            c => println!("? {:?}", c),
        }
        Ok(())
    }

    fn respond_to(&self, text: &str) -> Result<()> {
        let text = text.trim();
        if !text.starts_with("!") {
            return Ok(());
        }
        let words: Vec<&str> = text.split(' ').filter(|&w| !w.is_empty()).collect();
        let brain = self.brain.lock().expect("brain poisoning");
        let reply = brain.dispatch(&words).unwrap_or_else(errors::have_a_cow);
        println!("<{}> {}", brain.nick, reply);
        let cmd = Command::PRIVMSG(self.channel.clone(), reply);
        self.irc.send(cmd)
            .chain_err(|| format!("couldn't respond to {:?}", text))
    }
}

mod errors {
    use irc;
    use postgres;
    use rand;

    error_chain! {
        errors { NoIdea NoResult }
        foreign_links {
            Irc(irc::error::Error);
            Postgres(postgres::error::Error);
        }
    }

    pub fn have_a_cow<'a>(e: Error) -> String {
        match e {
            Error(ErrorKind::NoIdea, _) => no_idea().into(),
            Error(ErrorKind::NoResult, _) => "I got nothin'.".into(),
            e => format!("{}", e),
        }
    }

    fn no_idea() -> &'static str {
        let dunno = ["Huh?", "Don't remember that one.", "What's that?", "Hmm...", "Beats me."];
        dunno[rand::random::<usize>() % dunno.len()]
    }
}

mod rq {
    use postgres::{self, Connection};
    use errors::*;

    pub fn random_quote(sql: &Connection, nick: Option<&str>) -> Result<String> {

        let attempt = || -> postgres::Result<Option<String>> {
            let rows = if let Some(ref nick) = nick {
                sql.query(
                    "SELECT quote FROM quotegrabs
                     WHERE lower(nick) = lower($1)
                     ORDER BY random() LIMIT 1", // gah, slow scan, non-uniform to boot!
                    &[nick])?
            } else {
                sql.query(
                    "SELECT quote FROM quotegrabs
                     OFFSET random() * (SELECT COUNT(*) FROM quotegrabs) LIMIT 1",
                    &[])?
            };

            Ok(rows.iter().next().map(|row| row.get(0)))
        };

        for _ in 0..3 {
            if let Some(grab) = attempt()? {
                return Ok(grab);
            }
        }
        Err(ErrorKind::NoResult.into())
    }
}

struct Brain {
    sql: Connection,
    nick: String,
}

impl Brain {
    pub fn load() -> Brain {
        let url = env::var("DATABASE_URL").ok().expect("Missing DATABASE_URL");
        let sql = Connection::connect(&url[..], TlsMode::None).unwrap();
        let nick = "cantide".into();
        Brain { sql, nick }
    }

    fn dispatch(&self, words: &[&str]) -> Result<String> {
        let cmd = words[0];
        let a = words.get(1).map(|&word| word);

        let rq = |nick: Option<&str>| {
            rq::random_quote(&self.sql, nick).map(|quote| {
                if quote.starts_with('<') && quote.contains('>') {
                    let skip = quote.find('>').unwrap() + 2;
                    format!("◇ {}", &quote[skip..])
                } else if quote.starts_with("* ") {
                    let skip = quote[2..].find(' ').unwrap() + 3;
                    let actor = if rand::random() { "💃" } else { "🕺" };
                    format!("{} {}", actor, &quote[skip..])
                } else {
                    quote
                }
            })
        };
        match cmd {
            "!rq" => rq(a),
            "!!rq" => Ok(format!("{} {} {}", rq(a)?, rq(a)?, rq(a)?)),
            _ => Err(ErrorKind::NoIdea.into()),
        }
    }
}

fn main() {
    let host = "irc.opera.com";
    let channel = env::var("IRC_CHANNEL").ok().expect("Missing IRC_CHANNEL");
    println!("Connecting to {}...", host);

    let brain = Brain::load();

    let server = {
        let config = Config {
            nickname: Some(brain.nick.clone()),
            alt_nicks: Some(vec!["canti".to_string()]),
            server: Some(host.to_string()),
            channels: Some(vec![channel.clone()]),
            ..Default::default()
        };
        let server = IrcServer::from_config(config).unwrap();
        server.identify().unwrap();
        server
    };

    let ref mut cantide = Cantide {
        brain: Mutex::new(brain),
        channel,
        irc: server,
    };

    if let Err(e) = cantide.serve() {
        let stderr = &mut io::stderr();
        let oops = "couldn't write to stderr";

        writeln!(stderr, "fatal error: {}", e).expect(oops);
        for e in e.iter().skip(1) {
            writeln!(stderr, "  caused by: {}", e).expect(oops);
        }
        std::process::exit(1);
    }
}
