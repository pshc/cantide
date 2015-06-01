extern crate chrono;
extern crate irc;
extern crate postgres;

use std::env;
use std::default::Default;
use irc::client::prelude::*;
use irc::client::server::NetIrcServer;

struct Cantide {
    brain: Brain,
    channel: String,
    irc: NetIrcServer,
    _nick: String,
}

impl Cantide {
    pub fn serve(&self) {
        for msg in self.irc.iter() {
            self.handle(msg.unwrap())
        }
    }

    pub fn handle(&self, msg: Message) {
        match &msg.command[..] {
            "PING" => return,
            "353" | "366" => return,
            _ => ()
        }

        let nick = match msg.get_source_nickname() {
            Some(nick) => nick.to_string(), // is this really necessary?
            None => {
                println!("n?: {:?}", msg);
                return
            }
        };

        let is_normal_chat = msg.command == "PRIVMSG" && msg.args[0] == self.channel &&
                             msg.suffix.is_some();
        if is_normal_chat {
            let text = msg.suffix.unwrap();
            println!("<{}> {}", nick, text);
            if let Some(reply) = self.dispatch(&text) {
                let cmd = Command::PRIVMSG(self.channel.clone(), reply);
                self.irc.send(cmd).unwrap();
            }
        }
        else {
            println!("{:?}", msg)
        }
    }

    // maybe this ought to just return a &'a str... or call a closure or something?
    fn dispatch(&self, text: &str) -> Option<String> {
        if text.trim() == "!rq" {
            let quote = rq::random_quote(&self.brain.sql).map(|q| q.quote);
            return quote; //.expect("<missing>");
        }
        None
    }
}

mod types {
    use postgres;

    pub type Awake = postgres::Connection;
    //pub type Recall<'conn> = postgres::Statement<'conn>;
    //pub type Blueprint<'conn> = postgres::Result<Recall<'conn>>;

    pub type Hostmask = String;
}

mod rq {
    use chrono::{DateTime, UTC};
    use postgres;
    use types::*;

    pub struct Grab {
        pub nick: String,
        pub added_by: Hostmask,
        pub added_at: DateTime<UTC>,
        pub quote: String,
    }

    /*
    pub fn prepare(sql: &Awake) -> Blueprint {
        sql.prepare("SELECT nick, added_by, added_at, quote FROM quotegrabs
                     OFFSET random() * (SELECT COUNT(*) FROM quotegrabs) LIMIT 1")
    }
    */

    //pub fn random_quote(recall: &Recall) -> Option<Grab> {
    pub fn random_quote(sql: &Awake) -> Option<Grab> {

        // TODO: save this preparation
        let recall = sql.prepare(
                "SELECT nick, added_by, added_at, quote FROM quotegrabs
                 OFFSET random() * (SELECT COUNT(*) FROM quotegrabs) LIMIT 1").unwrap();

        let attempt = || -> postgres::Result<Option<Grab>> {
            let rows = try!(recall.query(&[]));
            Ok(rows.iter().next().map(|row| {
                Grab {
                    nick: row.get(0),
                    added_by: row.get(1),
                    added_at: row.get(2),
                    quote: row.get(3),
                }
            }))
        };

        let tries = 3;
        let mut complained = false;
        for _ in 0..tries {
            match attempt() {
                Ok(Some(grab)) => return Some(grab),
                Ok(None)       => (),
                Err(e)         => {
                    println!("random_quote: {}", e);
                    complained = true
                }
            }
        }
        if !complained {
            println!("couldn't get a random quote after {} tries", tries);
        }
        None
    }
}


struct Brain {
    sql: types::Awake,
    //rq: &'a types::Recall<'a>,
}

impl Brain {
    pub fn load() -> Brain {
        use postgres::{Connection, SslMode};
        let url = env::var("DATABASE_URL").ok().expect("Missing DATABASE_URL");
        let sql = Connection::connect(&url[..], &SslMode::None).unwrap();

        //let rq = rq::prepare(sql).unwrap();
        Brain {
            sql: sql,
            //rq: &rq,
        }
    }
}

fn main() {
    let host = "irc.opera.com";
    println!("Connecting to {}...", host);

    let brain = Brain::load();

    let server = {
        let config = Config {
            nickname: Some("cantide".to_string()),
            alt_nicks: Some(vec!["canti".to_string()]),
            server: Some(host.to_string()),
            channels: Some(vec!["#uweng".to_string()]),
            .. Default::default()
        };
        let server = IrcServer::from_config(config).unwrap();
        server.identify().unwrap();
        server
    };

    // wait until joined
    let mut nick = None;
    let mut channel = None;
    for m in server.iter() {
        let msg = m.unwrap();
        let ref cmd = msg.command;
        if cmd == "JOIN" {
            nick = msg.get_source_nickname().map(|s| s.to_string());
            channel = msg.suffix.clone();
            break // motd is over
        }
        else if cmd.starts_with("4") || cmd.starts_with("5") {
            println!("{:?}", msg) // error
        }
    }

    let nick = nick.expect("Who am I?").to_string();
    let channel = channel.expect("Where am I?").to_string();
    let ref mut cantide = Cantide {
        brain: brain,
        channel: channel,
        irc: server,
        _nick: nick,
    };
    cantide.serve();
}
