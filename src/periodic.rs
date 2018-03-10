extern crate clap;
extern crate futures;
extern crate serde;
extern crate serde_yaml;
extern crate tokio_core;
extern crate tokio_process;

use std::collections::HashMap;
use std::error::Error;
use std::fs::File;
use std::io::ErrorKind;
use std::io::prelude::*;
use std::path::Path;
use std::process::Command;
use std::rc::Rc;
use std::str;
use std::sync::RwLock;
use std::time::Duration;

use clap::{App, Arg, ArgMatches};
use futures::{future, Future, Stream};
#[macro_use]
extern crate serde_derive;
use tokio_core::reactor::{Core, Handle, Interval};
use tokio_process::CommandExt;

const VERSION: &'static str = "0.0.1";
const DEFAULT_CONTROL_FILE: &'static str = "./control.yaml";
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

enum TaskStatus {
    Running,
    Paused,
    Stopping,
}

#[derive(Debug, Deserialize, PartialEq)]
#[allow(non_camel_case_types)]
enum TaskControl {
    run,
    pause,
    stop,
}

fn default_name() -> String { String::from(DEFAULT_NAME) }
fn default_interval_secs() -> u64 { DEFAULT_INTERVAL_SECS.parse::<u64>().unwrap() }
fn default_max_concurrent() -> u32 { DEFAULT_MAX_CONCURRENT.parse::<u32>().unwrap() }

fn get_list(list_filename: &str) -> Option<HashMap<String, TaskControl>> {

    let list_path = Path::new(list_filename);
    if ! list_path.is_file() {
        None
    } else {
        match File::open(list_path) {
            Err(err) => {
                println!("couldn't open {:?} ({})", list_path, err.description());
                None
            },
            Ok(mut file) => {
                let mut yaml = String::new();
                match file.read_to_string(&mut yaml) {
                    Err(err) => {
                        println!("couldn't read {:?}: {}", list_path, err.description());
                        None
                    },
                    Ok(_) => {
                        match serde_yaml::from_str::<HashMap<String, TaskControl>>(&yaml) {
                            Ok(task_names) => Some(task_names),
                            Err(e) => {
                                println!("{}", e.description());
                                None
                            }
                        }
                    },
                }
            },
        }
    }
}

fn get_monitor_future(concurrent_counts: Rc<RwLock<HashMap<String, u32>>>,
                      handle: Handle) -> Box<Future<Item=(), Error=std::io::Error>> {

    let interval = Interval::new(Duration::from_secs(1), &handle).unwrap();
    Box::new(interval.for_each(move |_| {
        match get_list(DEFAULT_CONTROL_FILE) {
            Some(task_list) => {
                let counts_mut = concurrent_counts.read().unwrap();
                let mut remaining_tasks = counts_mut.len();
                for (name, &count) in counts_mut.iter() {
                    if let Some(control) = task_list.get(name) {
                        if count == 0 && *control == TaskControl::stop {
                            remaining_tasks -= 1;
                        }
                    }
                }
                match remaining_tasks {
                    0 => return Err(std::io::Error::new(ErrorKind::Interrupted, "all tasks stopped")),
                    _ => Ok(())
                }
            },
            None => Ok(()),
        }
    }))
}

fn get_task_future(task: PeriodicTask, concurrent_counts: Rc<RwLock<HashMap<String, u32>>>,
                   handle: Handle) -> Box<Future<Item=(), Error=std::io::Error>> {

    {
        let mut counts_mut = concurrent_counts.write().unwrap();
        counts_mut.insert(task.name.clone(), 0);
    }

    let interval = Interval::new(Duration::from_secs(task.interval_secs), &handle).unwrap();

    Box::new(interval.for_each(move |_| {

        match get_task_status(&task.name) {
            TaskStatus::Running => {
                let mut counts_mut = concurrent_counts.write().unwrap();
                let mut cur = counts_mut.get_mut(&task.name).unwrap();
                if  *cur < task.max_concurrent {
                    *cur += 1;
                    if *cur > 1 {
                        println!("invoking additional \"{}\" ({} now running)", task.name, *cur);
                    }
                    let cmd_array = task.cmd.split_whitespace().collect::<Vec<&str>>();
                    let concurrent_counts_clone = concurrent_counts.clone();
                    let task_name = task.name.clone();

                    match Command::new(&cmd_array[0]).args(cmd_array[1..].into_iter()).spawn_async(&handle) {
                        Ok(command) => {
                            handle.spawn(command.map(|_| { (task_name, concurrent_counts_clone) })
                                .then(|args| {
                                    let (task_name, concurrent_counts) = args.unwrap();
                                    let mut counts_mut = concurrent_counts.write().unwrap();
                                    let cur = counts_mut.get_mut(&task_name).unwrap();
                                    *cur -= 1;
                                    if *cur > 0 {
                                        println!("\"{}\" finished, {} still running", task_name, *cur);
                                    }
                                    future::ok(())
                                }))
                        },
                        Err(e) =>  {
                            println!("couldn't start \"{}\": {}", task.name, e);
                            let mut counts_mut = concurrent_counts.write().unwrap();
                            let mut cur = counts_mut.get_mut(&task.name).unwrap();
                            *cur -= 1;
                        }
                    }
                } else {
                    println!("not invoking \"{}\", max concurrent invocations ({}) reached",
                             task.name, task.max_concurrent);
                }
            },
            TaskStatus::Paused => println!("\"{}\" is paused", task.name),
            TaskStatus::Stopping => println!("\"{}\" will stop after all invocations finish", task.name),
        }
        future::ok(())
    }))
}

fn get_task_status(task_name: &str) -> TaskStatus {

    match get_list(DEFAULT_CONTROL_FILE) {
        Some(tasks) => {
            if let Some(control) = tasks.get(task_name) {
                match control {
                    &TaskControl::run => TaskStatus::Running,
                    &TaskControl::pause => TaskStatus::Paused,
                    &TaskControl::stop => TaskStatus::Stopping,
                }
            } else {
                TaskStatus::Running
            }
        },
        None => TaskStatus::Running
    }
}

fn run_futures_from_file(path: &str, concurrent_counts: Rc<RwLock<HashMap<String, u32>>>,
                         mut core: Core) {
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
                                tasks.push(get_task_future(task, concurrent_counts.clone(), core.handle()));
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

fn run_future_from_args(matches: ArgMatches, concurrent_counts: Rc<RwLock<HashMap<String, u32>>>,
                        mut core: Core) {
    if let Some(cmd) = matches.value_of("command") {
        let task = PeriodicTask { name: String::from(matches.value_of("name").unwrap()),
                                  interval_secs: matches.value_of("interval").unwrap().parse::<u64>().unwrap(),
                                  max_concurrent: matches.value_of("max-concurrent").unwrap().parse::<u32>().unwrap(),
                                  cmd: String::from(cmd)};
        let concurrent_counts_clone = concurrent_counts.clone();
        let monitor_future = get_monitor_future(concurrent_counts, core.handle());
        let task_future = get_task_future(task, concurrent_counts_clone, core.handle());
        
        drop(core.run(future::join_all(vec![monitor_future, task_future])))
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

    let concurrent_counts = Rc::new(RwLock::new(HashMap::<String,u32>::new()));
    let core = Core::new().unwrap();
    if matches.is_present("file") {
        run_futures_from_file(matches.value_of("file").unwrap(), concurrent_counts.clone(), core)
    } else {
        run_future_from_args(matches, concurrent_counts.clone(), core)
    }
}
