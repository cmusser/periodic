# periodic

Lightweight periodic command runner

## Introduction

This program executes one or more commands on a periodic basis. The
commands runs asynchronously with respect to the timer, which means
that the next invocation will happen at the desired interval, even if
the command is still running. Limits on the number of concurrently-running
copies of a process are enforced. One use of `periodic` is inside
Docker containers where something needs to happen repeatedly and you
don't want to go through the hassle of of setting up `cron`.

## Usage

There are two modes, one in which a single command is specified with
command line parameters and another in which one or more commands
are specified in a YAML-format configuration file.

### Command-line Specification Mode
Use the following options on the command line:

|Option|Description|Notes|
|---|---|---|
|`-i`|interval |The amount of time between each invocation, in seconds.|
|`-c`|command |Command to invoke. If it has arguments, quote the whole thing.|
|`-m`|max-concurrent|Maximum number of invocations allowed to launch.|
|`-n`|name|Descriptive name for the periodic task.|


#### Example:

	cargo run -- -i 2 -c 'ls -l /some/interesting/file'

### Configuration File Specification Mode

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
|name|Descriptive name for the periodic task.|

#### Example:

	cargo run -- -f periodic.yaml

## Pausing Tasks

If a file named `paused.yaml` exists in the periodic process' current
working directory and contains a YAML list of task names, tasks
matching these names will not be run at the timeout interval. You can
use this to pause some or all of the tasks specified in `periodic.yaml`.
The file is read for every interval for all tasks, so changes will take
effect the next time the task is scheduled to be run.

## Testing

### Scripts
The `test` directory contains some tests, which are referenced by both
`periodic-sample.yaml` and `paused-sample.yaml`.

- `print-delay.sh`: This sleeps a number of seconds and prints its
  start and end time. By default, it sleeps for 5 seconds. If a file
  named `delay.txt` exists in the same directory, it will read a
  numeric value from that file.

- `print-multiline-stdout-stderr.sh`: prints 5 lines, alternating
   between std stdout and stderr, sleeping a second between each.
