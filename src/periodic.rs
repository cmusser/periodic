extern crate chrono;
extern crate clap;
extern crate futures;
extern crate regex;
extern crate serde;
extern crate serde_yaml;
extern crate tokio_core;
extern crate tokio_process;
extern crate tokio_signal;

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

use chrono::prelude::*;
use clap::{App, Arg, ArgMatches};
use futures::{future, Future, Stream};
use regex::Regex;
#[macro_use]
extern crate serde_derive;
use tokio_core::reactor::{Core, Handle, Interval, Timeout};
use tokio_process::CommandExt;
use tokio_signal::unix::{Signal, SIGTERM, SIGUSR1, SIGUSR2};

const VERSION: &'static str = "1.0.0";
const DEFAULT_CONTROL_FILE: &'static str = "./control.yaml";
const DEFAULT_INTERVAL_SECS: &'static str = "5";
const DEFAULT_MAX_CONCURRENT: &'static str = "1";
const DEFAULT_NAME:&'static str = "periodic task";
const DAY_SECONDS:u64 = (60 * 60 * 24);

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

#[derive(Clone, Copy, Deserialize, PartialEq)]
#[allow(non_camel_case_types)]
enum TaskMode {
    run,
    pause,
    stop,
}

struct TaskState {
    pub concurrent_count: u32,
    pub mode: TaskMode,
}

impl TaskState {
    fn new() -> TaskState {
        TaskState { concurrent_count: 0, mode: TaskMode::run }
    }
}

struct TaskStateDb {
    tasks: RwLock<HashMap<String, TaskState>>,
}

impl TaskStateDb {
    fn new() -> TaskStateDb {
        TaskStateDb { tasks: RwLock::new(HashMap::new()) }
    }

    fn add_new_task(&self, task_name: &str) {
        let mut tasks_mut = self.tasks.write().unwrap();
        tasks_mut.insert(task_name.to_string(), TaskState::new());
    }

    fn set_all_task_modes(&self, mode: TaskMode) {
        let mut tasks_mut = self.tasks.write().unwrap();
        for ref mut task in tasks_mut.values_mut() {
            task.mode = mode
        }
    }

    fn set_task_modes_from_control_file(&self) {
        if let Some(control_tasks) = read_control_file(DEFAULT_CONTROL_FILE) {
            let mut tasks_mut = self.tasks.write().unwrap();
            for (task_name, task_mode) in control_tasks.iter() {
                if let Some(task) = tasks_mut.get_mut(task_name) {
                    task.mode = (*task_mode).clone();
                }
            }
        }
    }

    fn get_task_mode(&self, task_name: &str) -> TaskMode {
        let mut tasks_mut = self.tasks.write().unwrap();
        tasks_mut.get_mut(task_name).unwrap().mode
    }

    fn incr_concurrent_maybe(&self, task_name: &str, max_concurrent: u32) -> bool {
        let mut tasks_mut = self.tasks.write().unwrap();
        let task = tasks_mut.get_mut(task_name).unwrap();
        if task.concurrent_count < max_concurrent {
            task.concurrent_count += 1;
            if task.concurrent_count > 1 {
                println!("invoking additional \"{}\" ({} now running)", task_name, task.concurrent_count);
            }
            true
        } else {
            println!("not invoking \"{}\", max concurrent invocations ({}) reached",
                             task_name, max_concurrent);
            false
        }
    }

    fn decr_concurrent(&self, task_name: &str) -> u32 {
        let mut tasks_mut = self.tasks.write().unwrap();
        let task = tasks_mut.get_mut(task_name).unwrap();
        task.concurrent_count -= 1;
        task.concurrent_count
    }

    fn count_runnable(&self) -> usize {
        let tasks_mut = self.tasks.write().unwrap();
        let mut runnable = tasks_mut.len();
        for &ref task in tasks_mut.values() {
            if task.concurrent_count == 0 && task.mode == TaskMode::stop {
                runnable -= 1;
            }
        }
        runnable
    }
}

fn default_name() -> String { String::from(DEFAULT_NAME) }
fn default_interval_secs() -> u64 { DEFAULT_INTERVAL_SECS.parse::<u64>().unwrap() }
fn default_max_concurrent() -> u32 { DEFAULT_MAX_CONCURRENT.parse::<u32>().unwrap() }

fn read_control_file(list_filename: &str) -> Option<HashMap<String, TaskMode>> {

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
                        match serde_yaml::from_str::<HashMap<String, TaskMode>>(&yaml) {
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

fn get_signal_future(task_db: Rc<TaskStateDb>, signum: i32, mode: TaskMode,
                    handle: Handle) -> Box<Future<Item=(), Error=std::io::Error>> {
    Box::new(Signal::new(signum, &handle).flatten_stream().for_each(move |signal| {
        println!("signal {} received", signal);
        task_db.set_all_task_modes(mode);
        Ok(())
    }))
}

fn get_monitor_future(task_db: Rc<TaskStateDb>,
                      handle: Handle) -> Box<Future<Item=(), Error=std::io::Error>> {

    let interval = Interval::new(Duration::from_secs(1), &handle).unwrap();
    Box::new(interval.for_each(move |_| {
        task_db.set_task_modes_from_control_file();
        match task_db.count_runnable() {
            0 => {
                println!("exiting, all tasks have finished");
                return Err(std::io::Error::new(ErrorKind::Interrupted, "done"))
            },
            _ => Ok(())
        }
    }))
}

fn invoke_command(task: &PeriodicTask, task_db: &Rc<TaskStateDb>, handle: &Handle) {
    if task_db.incr_concurrent_maybe(&task.name, task.max_concurrent) {
        let cmd_array = task.cmd.split_whitespace().collect::<Vec<&str>>();
        let task_db_clone = task_db.clone();
        let task_name = task.name.clone();

        match Command::new(&cmd_array[0]).args(cmd_array[1..].into_iter()).spawn_async(&handle) {
            Ok(command) => {
                handle.spawn(command.map(|_| { (task_name, task_db_clone) })
                             .then(|args| {
                                 let (task_name, task_db) = args.unwrap();
                                 let count = task_db.decr_concurrent(&task_name);
                                 if count > 0 {
                                     println!("\"{}\" finished, {} still running", task_name, count);
                                 }
                                 future::ok(())
                             }))
            },
            Err(e) =>  {
                println!("couldn't start \"{}\": {}", task.name, e);
                task_db.decr_concurrent(&task.name);
            }
        }
    }
}

fn get_task_future(task: PeriodicTask, task_db: Rc<TaskStateDb>,
                   handle: Handle, start_delay: Duration) -> Box<Future<Item=(), Error=std::io::Error>> {

    let start_timeout: Timeout = Timeout::new(start_delay, &handle).unwrap();

    task_db.add_new_task(&task.name);

    if start_delay.as_secs() > 0 {
        println!("starting in {}", start_delay.as_secs());
    }
    Box::new(start_timeout.and_then(|_| {
        invoke_command(&task, &task_db, &handle);
        let interval = Interval::new(Duration::from_secs(task.interval_secs), &handle).unwrap();
        interval.for_each(move |_| {
            match task_db.get_task_mode(&task.name) {
                TaskMode::run => invoke_command(&task, &task_db, &handle),
                TaskMode::pause => println!("\"{}\" is paused", task.name),
                TaskMode::stop => {},
            }
            future::ok(())
        })
    }))
}

fn run_futures_from_file(path: &str, task_db: Rc<TaskStateDb>, mut core: Core, start_delay: Duration) {
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
                            tasks.push(get_monitor_future(task_db.clone(), core.handle()));
                            tasks.push(get_signal_future(task_db.clone(), SIGUSR1, TaskMode::pause, core.handle()));
                            tasks.push(get_signal_future(task_db.clone(), SIGUSR2, TaskMode::run, core.handle()));
                            tasks.push(get_signal_future(task_db.clone(), SIGTERM, TaskMode::stop, core.handle()));
                            for task in tasks_descriptions {
                                tasks.push(get_task_future(task, task_db.clone(), core.handle(), start_delay));
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

fn run_future_from_args(matches: ArgMatches, task_db: Rc<TaskStateDb>, mut core: Core, start_delay: Duration) {
    if let Some(cmd) = matches.value_of("command") {
        let task = PeriodicTask { name: String::from(matches.value_of("name").unwrap()),
                                  interval_secs: matches.value_of("interval").unwrap().parse::<u64>().unwrap(),
                                  max_concurrent: matches.value_of("max-concurrent").unwrap().parse::<u32>().unwrap(),
                                  cmd: String::from(cmd)};
        let futures = vec![get_monitor_future(task_db.clone(), core.handle()),
                           get_signal_future(task_db.clone(), SIGUSR1, TaskMode::pause, core.handle()),
                           get_signal_future(task_db.clone(), SIGUSR2, TaskMode::run, core.handle()),
                           get_signal_future(task_db.clone(), SIGTERM, TaskMode::stop, core.handle()),
                           get_task_future(task, task_db.clone(), core.handle(), start_delay)];

        drop(core.run(future::join_all(futures)))
    } else {
        println!("command not specified")
    }
}

fn get_start_delay_from_next(next: &str) -> Result<Duration, String> {

    let re = Regex::new(r"(?P<interval>hour|minute)(\+(?P<after>\d{1,2}))?$").unwrap();
    match re.captures(next) {
        Some(time) => {
            let after = match time.name("after") {
                Some(num) => num.as_str().parse::<u32>().unwrap(),
                None => 0,
            };
            let now = Local::now();
            let start_at = if &time["interval"] == "hour" {
                let next = now + chrono::Duration::hours(1);
                Local::today().and_hms(next.hour(), after, 0)
            } else {
                let next = now + chrono::Duration::minutes(1);
                Local::today().and_hms(next.hour(), next.minute(), after)
            };
            let start_delay = match start_at.signed_duration_since(now).num_seconds() {
                diff if diff >= 0 => diff as u64,
                diff => DAY_SECONDS - (diff.abs() as u64),
            };
            Ok(Duration::from_secs(start_delay))
        },
        None => Err(String::from(format!("invalid format for start time: {}", next)))
    }
}

fn get_start_delay_from_hh_mm(start_at: &str) -> Result<Duration, String> {

    let re = Regex::new(r"(?P<hour>\d{2}):(?P<minute>\d{2})").unwrap();
    match re.captures(start_at) {
        Some(time) => {
            let now = Utc::now();
            if let Some(start_at) = Local::today().and_hms_opt((&time["hour"]).parse::<u32>().unwrap(),
                                                               (&time["minute"]).parse::<u32>().unwrap(), 0) {
                let start_delay = match start_at.signed_duration_since(now).num_seconds() {
                    diff if diff >= 0 => diff as u64,
                    diff => DAY_SECONDS - (diff.abs() as u64) ,
                };
                Ok(Duration::from_secs(start_delay))
            } else {
                Err(String::from(format!("invalid values found in start time: {}", start_at)))
            }
        },
        None => Err(String::from(format!("invalid format for start time: {}", start_at)))
    }
}

fn main() {

    let matches = App::new("periodic")
        .version(VERSION)
        .author("Chuck Musser <cmusser@sonic.net>")
        .about("run commands periodically")
        .arg(Arg::with_name("file").empty_values(false)
             .short("f").long("file")
             .help("YAML file containing task descriptions. If set, overrides task-related args."))
        .arg(Arg::with_name("command").empty_values(false)
             .short("c").long("command")
             .help("command to run periodically' (required if no command file specified)"))
        .arg(Arg::with_name("interval").empty_values(false)
             .short("i").long("interval").default_value(DEFAULT_INTERVAL_SECS)
             .help("interval, in seconds, between command invocations"))
        .arg(Arg::with_name("max-concurrent").empty_values(false)
             .short("m").long("max-concurrent").default_value(DEFAULT_MAX_CONCURRENT)
             .help("number of concurrent invocations allowed"))
        .arg(Arg::with_name("name").empty_values(false)
             .short("n").long("name").default_value(DEFAULT_NAME)
             .help("descriptive name for command"))
        .arg(Arg::with_name("start-time").short("s").long("start-time").takes_value(true)
             .help(concat!("start time for tasks, either \"HH:MM\" for an absolute time ",
                   "or \"hour(+MM)\" or \"minute(+SS)\" to start at the next hour ",
                   "or minute, with an optional extra delay. Defaults to now.")))
        .get_matches();

    let start_delay = match matches.value_of("start-time") {
        Some(value) => get_start_delay_from_next(value).or_else(|_| get_start_delay_from_hh_mm(value)),
        None => Ok(Duration::from_secs(0)),
    };

    match start_delay {
         Ok(start_delay) => {
             let task_db = Rc::new(TaskStateDb::new());
             let core = Core::new().unwrap();
             if matches.is_present("file") {
                 run_futures_from_file(matches.value_of("file").unwrap(), task_db.clone(), core, start_delay)
             } else {
                 run_future_from_args(matches, task_db.clone(), core, start_delay)
             }
         },
        Err(e) => println!("{}", e),
    }
}
