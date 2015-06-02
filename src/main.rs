extern crate chrono;
extern crate irc;
extern crate postgres;
extern crate rand;

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
            println!("ok: {:?}", msg);
            let text = msg.suffix.unwrap();
            println!("<{}> {}", nick, text);
            self.respond_to(&text);
        }
        else {
            println!("nopers: {:?}", msg)
        }
    }

    fn respond_to(&self, text: &str) {
        let text = text.trim();
        if !text.starts_with("!") {
            return
        }
        let words: Vec<&str> = text.split(' ').filter(|&w| !w.is_empty()).collect();
        let reply = self.dispatch(&words).unwrap_or_else(|e| format!("{}", e));
        let cmd = Command::PRIVMSG(self.channel.clone(), reply);
        self.irc.send(cmd).unwrap();
    }

    fn dispatch(&self, words: &[&str]) -> types::R<String> {
        let n = words.len();
        let cmd = words[0];
        // TEMP should be `let a = words.get(1);` ish
        let a = if n > 1 { Some(words[1]) } else { None };

        let rq = |nick: Option<&str>| {
            rq::random_quote(&self.brain.sql, nick).map(|grab| grab.quote)
        };
        match cmd {
            "!rq"  => rq(a),
            "!!rq" => Ok(format!("{} {} {}", try!(rq(a)), try!(rq(a)), try!(rq(a)))),
            _      => Err(types::NoIdea),
        }
    }
}

mod types {
    use postgres;
    use rand;
    use std::fmt;
    pub use self::Whoops::*;

    pub type Awake = postgres::Connection;
    //pub type Recall<'conn> = postgres::Statement<'conn>;
    //pub type Blueprint<'conn> = postgres::Result<Recall<'conn>>;

    pub type Hostmask = String;

    pub enum Whoops {
        NoIdea,
        NoResult,
        BrainProblems,
    }

    pub type R<T> = Result<T, Whoops>;

    impl fmt::Display for Whoops {
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str(match *self {
                NoIdea => no_idea(),
                NoResult => "I got nothin'.",
                BrainProblems => "I got brain problems!",
            })
        }
    }

    impl From<postgres::error::Error> for Whoops {
        fn from(e: postgres::error::Error) -> Whoops {
            use std::error::Error;
            println!("pg: {}", e);
            if let Some(cause) = e.cause() {
                println!("cause: {}", cause);
            }
            BrainProblems
        }
    }

    fn no_idea() -> &'static str {
        let dunno = ["Huh?", "Don't remember that one.", "What's that?", "Hmm...", "Beats me."];
        dunno[rand::random::<usize>() % dunno.len()]
    }
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

    pub fn random_quote(sql: &Awake, nick: Option<&str>) -> R<Grab> {
        let anyone = nick.is_none();

        // TODO: save these preparations?
        //       use the macros?
        let recall = try!(sql.prepare(if anyone {
            "SELECT nick, added_by, added_at, quote FROM quotegrabs
             OFFSET random() * (SELECT COUNT(*) FROM quotegrabs) LIMIT 1"
        }
        else {
            "SELECT nick, added_by, added_at, quote FROM quotegrabs
             WHERE lower(nick) = lower($1)
             ORDER BY random() LIMIT 1" // gah, slow scan, non-uniform to boot!
        }));

        let attempt = || -> postgres::Result<Option<Grab>> {
            let rows = try!(if anyone {
                recall.query(&[])
            }
            else {
                let nick = nick.as_ref().unwrap();
                recall.query(&[nick])
            });

            Ok(rows.iter().next().map(|row| {
                Grab {
                    nick: row.get(0),
                    added_by: row.get(1),
                    added_at: row.get(2),
                    quote: row.get(3),
                }
            }))
        };

        for _ in 0..3 {
            if let Some(grab) = try!(attempt()) {
                return Ok(grab)
            }
        }
        Err(Whoops::NoResult)
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
    let channel = env::var("IRC_CHANNEL").ok().expect("Missing IRC_CHANNEL");
    println!("Connecting to {}...", host);

    let brain = Brain::load();

    let server = {
        let config = Config {
            nickname: Some("cantide".to_string()),
            alt_nicks: Some(vec!["canti".to_string()]),
            server: Some(host.to_string()),
            channels: Some(vec![channel]),
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
