#![recursion_limit = "1024"]

#[macro_use]
extern crate error_chain;
extern crate irc;
extern crate postgres;
extern crate rand;

use std::env;
use std::default::Default;
use std::io::{self, Write};

use irc::client::prelude::*;
use postgres::{Connection, TlsMode};

use errors::*;

struct Cantide {
    brain: Brain,
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
            Command::JOIN(chan, _, _) => println!("Joined {}", chan),
            Command::NOTICE(src, msg) => println!("Notice from {}: {:?}", src, msg),
            Command::PING(_, _) => (),
            Command::PRIVMSG(ref target, ref text) if target == &self.channel => {
                let nick = msg.source_nickname().ok_or_else(|| Error::from("nick missing"))?;
                println!("<{}> {}", nick, text);
                self.respond_to(text)?;
            }
            Command::Response(resp, args, text) => {
                use irc::proto::response::Response::*;
                match resp {
                    RPL_ISUPPORT | RPL_MOTDSTART | RPL_MOTD | RPL_ENDOFMOTD => (),
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
        let reply = self.dispatch(&words).unwrap_or_else(errors::have_a_cow);
        println!("< {}", reply);
        let cmd = Command::PRIVMSG(self.channel.clone(), reply);
        self.irc.send(cmd)
            .chain_err(|| format!("couldn't respond to {:?}", text))
    }

    fn dispatch(&self, words: &[&str]) -> Result<String> {
        let cmd = words[0];
        let a = words.get(1).map(|&word| word);

        let rq = |nick: Option<&str>| {
            rq::random_quote(&self.brain.sql, nick).map(|quote| {
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

    /*
    pub fn prepare(sql: &Connection) -> Blueprint {
        sql.prepare("SELECT nick, added_by, added_at, quote FROM quotegrabs
                     OFFSET random() * (SELECT COUNT(*) FROM quotegrabs) LIMIT 1")
    }
    */

    pub fn random_quote(sql: &Connection, nick: Option<&str>) -> Result<String> {
        // TODO: save these preparations?
        //       use the macros?
        let recall = sql.prepare(if nick.is_none() {
            "SELECT quote FROM quotegrabs
             OFFSET random() * (SELECT COUNT(*) FROM quotegrabs) LIMIT 1"
        } else {
            "SELECT quote FROM quotegrabs
             WHERE lower(nick) = lower($1)
             ORDER BY random() LIMIT 1" // gah, slow scan, non-uniform to boot!
        })?;

        let attempt = || -> postgres::Result<Option<String>> {
            let rows = if let Some(ref nick) = nick {
                recall.query(&[nick])?
            } else {
                recall.query(&[])?
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
    //rq: &'a types::Recall<'a>,
}

impl Brain {
    pub fn load() -> Brain {
        let url = env::var("DATABASE_URL").ok().expect("Missing DATABASE_URL");
        let sql = Connection::connect(&url[..], TlsMode::None).unwrap();

        //let rq = rq::prepare(sql).unwrap();
        Brain {
            sql: sql,
            //rq: &rq,
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
            nickname: Some("cantide".to_string()),
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
        brain: brain,
        channel: channel,
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
