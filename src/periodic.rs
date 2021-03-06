extern crate clap;
extern crate chrono;
extern crate futures;
extern crate periodic;
extern crate serde;
extern crate serde_yaml;
extern crate tokio_core;
extern crate tokio_process;
extern crate tokio_signal;

use std::collections::HashMap;
use std::error::Error;
use std::fs::File;
use std::io::prelude::*;
use std::io::ErrorKind;
use std::path::Path;
use std::process::{Command, ExitStatus};
use std::rc::Rc;
use std::str;
use std::sync::RwLock;
use std::time::Duration;

use clap::{crate_authors, crate_version, App, Arg, ArgMatches};
use chrono::prelude::*;
use futures::{future, Future, Stream};
#[macro_use]
extern crate serde_derive;
use serde::{Deserialize, Deserializer};
use tokio_core::reactor::{Core, Handle, Interval, Timeout};
use tokio_process::CommandExt;
use tokio_signal::unix::{Signal, SIGTERM, SIGUSR1, SIGUSR2};

const DEFAULT_CONTROL_FILE: &'static str = "./control.yaml";
const DEFAULT_INTERVAL_SECS: &'static str = "5";
const DEFAULT_MAX_CONCURRENT: &'static str = "1";
const DEFAULT_NAME: &'static str = "periodic task";

#[derive(Debug, Deserialize)]
struct PeriodicTask {
    #[serde(default = "default_name")]
    name: String,
    #[serde(default = "default_interval_secs")]
    interval_secs: u64,
    #[serde(default = "default_max_concurrent")]
    max_concurrent: u32,
    #[serde(deserialize_with = "cmd_from_config")]
    cmd: Vec<String>,
}

#[derive(Clone, Copy, Deserialize, PartialEq)]
#[allow(non_camel_case_types)]
enum TaskMode {
    run,
    pause,
    stop,
}

fn cmd_from_config<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;
    String::deserialize(deserializer).and_then(|string| {
        shellwords::split(&string).map_err(|_| Error::custom("Mismatched quotes"))
    })
}

struct TaskState {
    pub concurrent_count: u32,
    pub mode: TaskMode,
}

impl TaskState {
    fn new() -> TaskState {
        TaskState {
            concurrent_count: 0,
            mode: TaskMode::run,
        }
    }
}

struct TaskStateDb {
    propagate_sigterm_to_children: bool,
    tasks: RwLock<HashMap<String, TaskState>>,
    active_pids: RwLock<Vec<u32>>,
}

impl TaskStateDb {
    fn new() -> TaskStateDb {
        TaskStateDb {
            propagate_sigterm_to_children: false,
            tasks: RwLock::new(HashMap::new()),
            active_pids: RwLock::new(Vec::new()),
        }
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
        if mode == TaskMode::stop {
            let active_pids = self.active_pids.read().unwrap().clone();
            if self.propagate_sigterm_to_children {
                for pid in active_pids.into_iter() {
                    println!("sending SIGTERM to {}", pid);
                }
            } else {
                println!(
                    "waiting for PID(s): {}",
                    active_pids
                        .into_iter()
                        .map(|pid| format!("{}", pid))
                        .collect::<Vec<String>>()
                        .join(", ")
                );
            }
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

    fn init_process_if_allowed(&self, task_name: &str, max_concurrent: u32) -> bool {
        let mut tasks_mut = self.tasks.write().unwrap();
        let task = tasks_mut.get_mut(task_name).unwrap();
        if task.concurrent_count < max_concurrent {
            task.concurrent_count += 1;
            if task.concurrent_count > 1 {
                println!(
                    "invoking additional \"{}\" ({} now running)",
                    task_name, task.concurrent_count
                );
            }
            true
        } else {
            println!(
                "not invoking \"{}\", max concurrent invocations ({}) reached",
                task_name, max_concurrent
            );
            false
        }
    }

    fn start_process(&self, task_name: &str, pid: u32) {
        println!("PID {} started for {}", pid, task_name);
        self.active_pids.write().unwrap().push(pid);
    }

    fn finish_process(&self, task_name: &str, terminated_pid: u32, status: ExitStatus) {
        let mut tasks_mut = self.tasks.write().unwrap();
        let task = tasks_mut.get_mut(task_name).unwrap();

        let mut active_pids_mut = self.active_pids.write().unwrap();
        active_pids_mut.retain(|pid| *pid != terminated_pid);

        task.concurrent_count -= 1;
        let status_msg = match status.code() {
            Some(code) => format!(" (exit status {})", code),
            None => format!(" (terminated by signal)"),
        };
        println!(
            "\"{}\": PID {} terminated{}{}",
            task_name,
            terminated_pid,
            status_msg,
            if task.concurrent_count > 0 {
                format!(", {} still running", task.concurrent_count)
            } else {
                format!("")
            }
        );
    }

    fn cleanup_failed_process(&self, task_name: &str) {
        let mut tasks_mut = self.tasks.write().unwrap();
        let task = tasks_mut.get_mut(task_name).unwrap();
        task.concurrent_count -= 1;
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

fn default_name() -> String {
    String::from(DEFAULT_NAME)
}
fn default_interval_secs() -> u64 {
    DEFAULT_INTERVAL_SECS.parse::<u64>().unwrap()
}
fn default_max_concurrent() -> u32 {
    DEFAULT_MAX_CONCURRENT.parse::<u32>().unwrap()
}

fn read_control_file(list_filename: &str) -> Option<HashMap<String, TaskMode>> {
    let list_path = Path::new(list_filename);
    if !list_path.is_file() {
        None
    } else {
        match File::open(list_path) {
            Err(err) => {
                println!("couldn't open {:?} ({})", list_path, err.description());
                None
            }
            Ok(mut file) => {
                let mut yaml = String::new();
                match file.read_to_string(&mut yaml) {
                    Err(err) => {
                        println!("couldn't read {:?}: {}", list_path, err.description());
                        None
                    }
                    Ok(_) => match serde_yaml::from_str::<HashMap<String, TaskMode>>(&yaml) {
                        Ok(task_names) => Some(task_names),
                        Err(e) => {
                            println!("{}", e.description());
                            None
                        }
                    },
                }
            }
        }
    }
}

fn get_signal_future(
    task_db: Rc<TaskStateDb>,
    signum: i32,
    mode: TaskMode,
    handle: Handle,
) -> Box<dyn Future<Item = (), Error = std::io::Error>> {
    Box::new(
        Signal::new(signum, &handle)
            .flatten_stream()
            .for_each(move |signal| {
                println!("signal {} received", signal);
                task_db.set_all_task_modes(mode);
                Ok(())
            }),
    )
}

fn get_monitor_future(
    task_db: Rc<TaskStateDb>,
    handle: Handle,
) -> Box<dyn Future<Item = (), Error = std::io::Error>> {
    let interval = Interval::new(Duration::from_secs(1), &handle).unwrap();
    Box::new(interval.for_each(move |_| {
        task_db.set_task_modes_from_control_file();
        match task_db.count_runnable() {
            0 => {
                println!("exiting, all tasks have finished");
                return Err(std::io::Error::new(ErrorKind::Interrupted, "done"));
            }
            _ => Ok(()),
        }
    }))
}

fn invoke_command(task: &PeriodicTask, task_db: &Rc<TaskStateDb>, handle: &Handle) {
    if task_db.init_process_if_allowed(&task.name, task.max_concurrent) {
        let task_db_clone = task_db.clone();
        let (cmd_name, cmd_args) = (task.cmd[0].clone(), task.cmd[1..].into_iter());
        let task_name = task.name.clone();
        match Command::new(cmd_name).args(cmd_args).spawn_async(&handle) {
            Ok(command) => {
                let pid = command.id();
                task_db_clone.start_process(&task_name, pid);
                handle.spawn(
                    command
                        .map(move |status| (task_name, task_db_clone, pid, status))
                        .then(|args| {
                            let (task_name, task_db, pid, status) = args.unwrap();
                            task_db.finish_process(&task_name, pid, status);
                            future::ok(())
                        }),
                )
            }
            Err(e) => {
                println!("couldn't start \"{}\": {}", task.name, e);
                task_db.cleanup_failed_process(&task.name);
            }
        }
    }
}

fn get_task_future(
    task: PeriodicTask,
    task_db: Rc<TaskStateDb>,
    handle: Handle,
    start_delay: Duration,
) -> Box<dyn Future<Item = (), Error = std::io::Error>> {
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
                TaskMode::stop => {}
            }
            future::ok(())
        })
    }))
}

fn run_futures_from_file(
    path: &str,
    task_db: Rc<TaskStateDb>,
    mut core: Core,
    start_delay: Duration,
) {
    match File::open(path) {
        Err(err) => println!("couldn't open {} ({})", path, err.description()),
        Ok(mut file) => {
            let mut yaml = String::new();
            match file.read_to_string(&mut yaml) {
                Err(err) => println!("couldn't read {}: {}", path, err.description()),
                Ok(_) => match serde_yaml::from_str::<Vec<PeriodicTask>>(&yaml) {
                    Ok(tasks_descriptions) => {
                        let mut tasks: Vec<Box<dyn Future<Item = (), Error = std::io::Error>>> =
                            Vec::new();
                        tasks.push(get_monitor_future(task_db.clone(), core.handle()));
                        tasks.push(get_signal_future(
                            task_db.clone(),
                            SIGUSR1,
                            TaskMode::pause,
                            core.handle(),
                        ));
                        tasks.push(get_signal_future(
                            task_db.clone(),
                            SIGUSR2,
                            TaskMode::run,
                            core.handle(),
                        ));
                        tasks.push(get_signal_future(
                            task_db.clone(),
                            SIGTERM,
                            TaskMode::stop,
                            core.handle(),
                        ));
                        for task in tasks_descriptions {
                            tasks.push(get_task_future(
                                task,
                                task_db.clone(),
                                core.handle(),
                                start_delay,
                            ));
                        }
                        drop(core.run(future::join_all(tasks)))
                    }
                    Err(e) => println!("{}", e.description()),
                },
            }
        }
    }
}

fn run_future_from_args(
    matches: ArgMatches,
    task_db: Rc<TaskStateDb>,
    mut core: Core,
    start_delay: Duration,
) {
    if let Some(cmd) = matches.values_of("COMMAND") {
        let task = PeriodicTask {
            name: String::from(matches.value_of("name").unwrap()),
            interval_secs: matches
                .value_of("interval")
                .unwrap()
                .parse::<u64>()
                .unwrap(),
            max_concurrent: matches
                .value_of("max-concurrent")
                .unwrap()
                .parse::<u32>()
                .unwrap(),
            cmd: cmd.map(|arg| arg.to_string()).collect(),
        };
        let futures = vec![
            get_monitor_future(task_db.clone(), core.handle()),
            get_signal_future(task_db.clone(), SIGUSR1, TaskMode::pause, core.handle()),
            get_signal_future(task_db.clone(), SIGUSR2, TaskMode::run, core.handle()),
            get_signal_future(task_db.clone(), SIGTERM, TaskMode::stop, core.handle()),
            get_task_future(task, task_db.clone(), core.handle(), start_delay),
        ];

        drop(core.run(future::join_all(futures)))
    } else {
        println!("command not specified")
    }
}

fn main() {
    let matches = App::new("periodic")
        .version(crate_version!())
        .author(crate_authors!())
        .about("run commands periodically")
        .arg(
            Arg::with_name("file")
                .empty_values(false)
                .short("f")
                .long("file")
                .help(
                    "YAML file containing task descriptions. If set, overrides task-related args.",
                ),
        )
        .arg(
            Arg::with_name("COMMAND")
                .multiple(true)
                .help("command to run periodically' (required if no command file specified)"),
        )
        .arg(
            Arg::with_name("interval")
                .empty_values(false)
                .short("i")
                .long("interval")
                .default_value(DEFAULT_INTERVAL_SECS)
                .help("interval, in seconds, between command invocations"),
        )
        .arg(
            Arg::with_name("max-concurrent")
                .empty_values(false)
                .short("m")
                .long("max-concurrent")
                .default_value(DEFAULT_MAX_CONCURRENT)
                .help("number of concurrent invocations allowed"),
        )
        .arg(
            Arg::with_name("name")
                .empty_values(false)
                .short("n")
                .long("name")
                .default_value(DEFAULT_NAME)
                .help("descriptive name for command"),
        )
        .arg(
            Arg::with_name("start-time")
                .short("s")
                .long("start-time")
                .takes_value(true)
                .help(concat!(
                    "start time for tasks, either \"HH:MM\" for an absolute time ",
                    "or \"hour(+MM)\" or \"minute(+SS)\" to start at the next hour ",
                    "or minute, with an optional extra delay. Defaults to now."
                )),
        )
        .get_matches();

    let start_delay = match matches.value_of("start-time") {
        Some(value) => periodic::time::get_start_delay_from_next(Local::now(), value)
            .or_else(|_| periodic::time::get_start_delay_from_hh_mm(value)),
        None => Ok(Duration::from_secs(0)),
    };

    match start_delay {
        Ok(start_delay) => {
            let task_db = Rc::new(TaskStateDb::new());
            let core = Core::new().unwrap();
            if matches.is_present("file") {
                run_futures_from_file(
                    matches.value_of("file").unwrap(),
                    task_db.clone(),
                    core,
                    start_delay,
                )
            } else {
                run_future_from_args(matches, task_db.clone(), core, start_delay)
            }
        }
        Err(e) => println!("{}", e),
    }
}
