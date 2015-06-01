extern crate irc;
extern crate rusqlite;

use std::default::Default;
use irc::client::prelude::*;

struct Cantide {
    nick: String,
    channel: String,
}

impl Cantide {
    fn handle(&mut self, msg: Message) {
        if msg.command == "PING" {
            return
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
            println!("<{}> {}", nick, text)
        }
        else {
            println!("{:?}", msg)
        }
    }
}

type Awake = SqliteConnection
type Recall<'a> = SqliteStatement<'a>;
type Blueprint<'a> = SqliteResult<Recall<'a>>;

type Hostmask = String;

mod rq {
    type Date = String; // :(((((( use rust-chrono?
    struct Grab {
        nick: String,
        added_by: Hostmask,
        added_at: Date,
        quote: String,
    }

    fn prepare(sql: &Awake) -> Blueprint {
        sql.prepare("SELECT nick, added_by, date(added_at), quote FROM quotegrabs
                     WHERE rowid = (abs(random()) % (SELECT MAX(rowid)+1 FROM quotegrabs))")
    }

    fn random_quote(recall: &Recall) -> Option<Grab> {

        fn attempt() -> SqliteResult<Grab> {
            let mut rows = try!(recall.query(&[]));
            // this is wrong; should be graceful if no result
            let row = try!(rows.next().unwrap());
            Grab {
                nick: row.get(0),
                added_by: row.get(1),
                added_at: row.get(2),
                quote: row.get(3),
            }
        }

        for _attempt in ..3 {
            match attempt() {
                Ok(quote) => return Some(quote),
                Err(e)    => println!("random_quote: {}", e)
            }
        }
        None
    }
}


struct Brain {
    _awake: Awake,
    rq: Recall,
}

impl Brain {
    pub fn new() -> Brain {
        let sql = SqliteConnection::open("quotegrabs.sqlite");
        Brain {
            _awake: sql,
            rq: rq::prepare(sql).unwrap(),
        }
    }
}

fn main() {
    let host = "irc.opera.com";
    println!("Connecting to {}...", host);

    let server = {
        let config = Config {
            nickname: Some("cantide".to_string()),
            alt_nicks: Some(vec!["canti".to_string()]),
            server: Some(host.to_string()),
            channels: Some(vec!["#testbot".to_string()]),
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
    let ref mut cantide = Cantide {nick: nick, channel: channel};
    for msg in server.iter() {
        cantide.handle(msg.unwrap())
    }
}
