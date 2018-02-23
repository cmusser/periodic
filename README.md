# periodic

Simple periodic command runner

## Introduction

This program executes one or more commands on a periodic basis. The
commands runs asynchronously with respect to the timer, which means
that the next invocation will happen at the desired interval, even if
the command is still running. Limits on the number of concurrently-running
copies of a process are enforced. One use of `periodic` is inside
Dockers containers where something nees to happen repeatedly and you
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

	cargo run -- -i 2 'ls -l /some/interesting/file'

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


## Testing

There is a `test.sh` program you can use to try it out. This script simply
sleeps a number of seconds and prints its start and end time. By default, it
sleeps for 5 seconds. If a file named `delay.txt` exists in the same directory,
it will read a numeric value from  that file.

## Implementation Notes

### Enforcing a Limit on the Number of Concurrent Processes per "Task"

This demonstrates (or abuses) the technique capturing a variable that
started life in the scope of a function in a longer-lived scope: the
closure associated with a boxed future.

Inside the `get_future()` function, a count of concurrently running
processes is created, then explicitly moved into the `for_each()`
closure, so that its ownership is transferred to the longer-lasting
scope than begins when each periodic timeout occurs. It gets cloned
before the `Command` future is created (along with the name of the
task). WHen `then()` is called on the command, its closure captures
the variable. At that point the counter can be decremented. It's done
in this way so that the `for_each()` closture and the `then()` future
can share a reference to this value. The former increments the value
and the latter decrements it.

One possible simplification is moving the counter clone into the
command closure, rather than using `map()` to pass it in via an
argument.
