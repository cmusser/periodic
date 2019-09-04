# periodic

Lightweight periodic command runner

Linux: [![Build Status](https://travis-ci.org/cmusser/periodic.svg?branch=master)](https://travis-ci.org/cmusser/periodic)

## Introduction

This program executes one or more commands on a periodic basis. The
commands run asynchronously with respect to their timer, which means
that an already-invoked process won't prevent the next invocation from
starting on the desired interval. However, limits on the number of
concurrently-running copies of a process are enforced. One use of
`periodic` is inside Docker containers where something needs to happen
repeatedly and you don't want to go through the hassle of of setting
up `cron`.

## Usage

There are two ways to use `periodic`. A single command can be run via
command line arguments and one or more commands can be run if you
specify them in a YAML-format configuration file.

### Using Command-line Arguments
Use the following options on the command line:

|Option|Description|Notes|
|---|---|---|
|`-i`|interval |The amount of time between each invocation, in seconds.|
|`-m`|max-concurrent|Maximum number of invocations allowed to launch.|
|`-n`|name|Name for the periodic task.|
|`-s`|start-time|start time for tasks, either "HH:MM" for an absolute time or "hour[+MM]" or "minute[+SS]" to start at the next hour or minute, with an optional extra delay. Defaults to now.|
|COMMAND|The actual command to run, followed by its arguments.|This is specified in the same way as if the command were being run directly, with whitespace separated arguments. See the notes below on how to use commands that have flag arguments.|


#### Example:

	periodic -i 2 ls /some/interesting/file

If your command contains flags, place a `--` before the command so that `periodic` will not attempt to interpret those flags as its own. For example:

	periodic -i 2 -- date -u

**This is a change from earlier (1.x) versions of periodic that
required the command to be specified by a `-c` flag, enclosed in
quotes. The previous method made it harder to use from Docker, due to
the difficulty with retaining whitespace in the arguments to Docker's
`CMD` directive.**

### Using a Configuration File

Use the following option on the command line.

|Option|Description|Notes|
|---|---|---|
|`-f`|file |Path to YAML-format configuration file. This overrides all parameters specified above.|

The file must have a top-level array. Each element can have the following.
Of these, only `command` is required, although the defaults for the remaining
ones are probably not appropriate for real-world use.

|Attribute|Notes|
|---|---|
|interval |The amount of time between each invocation, in seconds.|
|command |Command to invoke.|
|max-concurrent|Maximum number of invocations allowed to launch.|
|name|Name for the periodic task.|

#### Example:

	periodic -f my-tasks.yaml

## Runtime Control

 Tasks can be in three modes, which can be changed dynamically:

- `run`: The task command is invoked regularly, on the specified
  interval. This is the default.

- `pause`: The task command will not be invoked on the interval, but
  existing ones will run to completion. Paused tasks can be restarted.

- `stop`: The task command will not be invoked on the interval, but
  existing ones will run to completion. This is similar to being
  paused, but it marks the task in a way that, once all tasks are in
  this mode and all invocations are completed, `periodic` will
  exit. The purpose of this is to provide a graceful shutdown of the
  tasks.

### File-based

If a file named `control.yaml` exists in the current working directory
of `periodic` and is of the form shown, below, the tasks modes can
be specified explicitly. Entries in the file looks like:
`task-name`: `<run|pause|stop>`

### Signal-based

As a convenience, the mode of all tasks can be controlled by sending a
signal to the `periodic` process:

- `SIGUSR1`: pause all tasks
- `SIGUSR2`: resume all tasks
- `SIGTERM`: stop all tasks.

## Test Scripts

The `test` directory contains some example scripts that can be run by
`periodic` in order to test it out. They are referenced by both
`periodic-sample.yaml` and `control-sample.yaml`.

- `print-delay.sh`: This sleeps a number of seconds and prints its
  start and end time. By default, it sleeps for 5 seconds but you
  can specify a different value as the first (and only) argument.

- `print-multiline-stdout-stderr.sh`: prints 5 lines, alternating
   between std stdout and stderr, sleeping a second between each.
