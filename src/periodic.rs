extern crate clap;
extern crate futures;
extern crate serde;
extern crate serde_yaml;
extern crate tokio_core;
extern crate tokio_process;

use std::cell::Cell;
use std::error::Error;
use std::fs::File;
use std::io::prelude::*;
use std::path::Path;
use std::process::Command;
use std::rc::Rc;
use std::str;
use std::time::Duration;

use clap::{App, Arg, ArgMatches};
use futures::{future, Future, Stream};
#[macro_use]
extern crate serde_derive;
use tokio_core::reactor::{Core, Handle, Interval};
use tokio_process::CommandExt;

const VERSION: &'static str = "0.0.4";
const DEFAULT_INTERVAL_SECS: &'static str = "5";
const DEFAULT_MAX_CONCURRENT: &'static str = "3";
const DEFAULT_NAME:&'static str = "periodic task";

#[derive(Debug, Deserialize)]
struct PeriodicTask {
    #[serde(default = "default_name")]
    name: String,
    #[serde(default = "default_interval_secs")]
    interval_secs: u64,
    #[serde(default = "default_max_concurrent")]
    max_concurrent: u32,
    cmd: String,
}

fn default_name() -> String { String::from(DEFAULT_NAME) }
fn default_interval_secs() -> u64 { DEFAULT_INTERVAL_SECS.parse::<u64>().unwrap() }
fn default_max_concurrent() -> u32 { DEFAULT_MAX_CONCURRENT.parse::<u32>().unwrap() }

fn task_paused(task_name: &str) -> bool {

    let paused_filename = Path::new("./paused.yaml");
    if ! paused_filename.is_file() {
        false
    } else {
        match File::open(paused_filename) {
            Err(err) => {
                println!("couldn't open {:?} ({})", paused_filename, err.description());
                false
            },
            Ok(mut file) => {
                let mut yaml = String::new();
                match file.read_to_string(&mut yaml) {
                    Err(err) => {
                        println!("couldn't read {:?}: {}", paused_filename, err.description());
                        false
                    },
                    Ok(_) => {
                        match serde_yaml::from_str::<Vec<String>>(&yaml) {
                            Ok(paused_tasks) => {
                                match paused_tasks.into_iter().find(|paused_task| { task_name == paused_task }) {
                                    Some(_) => true,
                                    None => false,
                                }
                            },
                            Err(e) => {
                                println!("{}", e.description());
                                false
                            }
                        }
                    },
                }
            },
        }
    }
}

fn get_future(task: PeriodicTask, handle: Handle) -> Box<Future<Item=(), Error=std::io::Error>> {
    let interval = Interval::new(Duration::from_secs(task.interval_secs), &handle).unwrap();

    let concurrent_count = Rc::new(Cell::new(0));

    Box::new(interval.for_each(move |_| {

        if !task_paused(&task.name) {
            let cur = concurrent_count.get();
            if  cur < task.max_concurrent {
                let new_count = cur + 1;
                concurrent_count.replace(new_count);
                if new_count > 1 {
                    println!("invoking additional \'{}\" ({} now running)", task.name, new_count);
                }
                let cmd_array = task.cmd.split_whitespace().collect::<Vec<&str>>();
                let counter_clone = concurrent_count.clone();
                let task_name = task.name.clone();

                match Command::new(&cmd_array[0]).args(cmd_array[1..].into_iter()).spawn_async(&handle) {
                    Ok(command) => {
                        let f = command.map(|_| { (task_name, counter_clone) })
                            .then(|args| {
                                let (task_name, counter_clone) = args.unwrap();
                                let new_count = counter_clone.get() - 1;
                                counter_clone.replace(new_count);
                                if new_count > 0 {
                                    println!("\"{}\" finished, {} still running", task_name, new_count);
                                }
                                future::ok(())
                            });
                        handle.spawn(f)
                    },
                    Err(e) =>  {
                        println!("couldn't start \"{}\": {}", task.name, e);
                        counter_clone.replace(counter_clone.get() - 1);
                    }
                }
            } else {
                println!("not invoking \"{}\", max concurrent invocations ({}) reached",
                         task.name, task.max_concurrent);
            }
        }
        future::ok(())
    }))
}

fn run_futures_from_file(path: &str, mut core: Core) {
    match File::open(path) {
        Err(err) => println!("couldn't open {} ({})", path, err.description()),
        Ok(mut file) => {
            let mut yaml = String::new();
            match file.read_to_string(&mut yaml) {
                Err(err) => println!("couldn't read {}: {}", path, err.description()),
                Ok(_) => {
                    match serde_yaml::from_str::<Vec<PeriodicTask>>(&yaml) {
                        Ok(tasks_descriptions) => {
                            let mut tasks: Vec<Box<Future<Item=(), Error=std::io::Error>>> = Vec::new();
                            for task in tasks_descriptions {
                                tasks.push(get_future(task, core.handle()));
                            }
                            drop(core.run(future::join_all(tasks)))
                        },
                        Err(e) => println!("{}", e.description())
                    }
                },
            }
        },
    }
}

fn run_future_from_args(matches: ArgMatches, mut core: Core) {
    if let Some(cmd) = matches.value_of("command") {
        let task = PeriodicTask { name: String::from(matches.value_of("name").unwrap()),
                                  interval_secs: matches.value_of("interval").unwrap().parse::<u64>().unwrap(),
                                  max_concurrent: matches.value_of("max-concurrent").unwrap().parse::<u32>().unwrap(),
                                  cmd: String::from(cmd)};
        let handle = core.handle();
        drop(core.run(get_future(task, handle)))
    } else {
        println!("command not specified")
    }
}

fn main() {

    let matches = App::new("periodic")
        .version(VERSION)
        .author("Chuck Musser <cmusser@sonic.net>")
        .about("run commands periodically")
        .arg(Arg::with_name("file")
             .short("f").long("file").takes_value(true)
             .help("YAML file containing task descriptions. If set, overrides all other args."))
        .arg(Arg::with_name("command").takes_value(true)
             .short("c").long("command")
             .help("command to run periodically' (required if no command file specified)"))
        .arg(Arg::with_name("interval")
             .short("i").long("interval").default_value(DEFAULT_INTERVAL_SECS)
             .help(""))
        .arg(Arg::with_name("max-concurrent")
             .short("m").long("max-concurrent").default_value(DEFAULT_MAX_CONCURRENT)
             .help("number of concurrent invocations allowed"))
        .arg(Arg::with_name("name")
             .short("n").long("name").default_value(DEFAULT_NAME)
             .help("descriptive name for command"))
        .get_matches();

    let core = Core::new().unwrap();
    if matches.is_present("file") {
        run_futures_from_file(matches.value_of("file").unwrap(), core)
    } else {
        run_future_from_args(matches, core)
    }
}
