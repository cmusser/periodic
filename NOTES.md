# Implementation Notes

## Enforcing a Limit on the Number of Concurrent Processes per "Task"

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
