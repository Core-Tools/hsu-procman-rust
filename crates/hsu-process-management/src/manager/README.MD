# ADR: Migrating Process Manager to an Actor-Based Architecture

## Context and Problem

The original implementation of `ProcessManager` was built around the following model:

* Shared mutable state:

    * `Arc<RwLock<HashMap<...>>>` for processes
    * `Arc<Mutex<State>>` for the manager state
* Multiple asynchronous contexts running in parallel:

    * public async API methods
    * a separate heartbeat task
* Execution of heavy operations (`start/stop/restart`) performed **under a lock**, with `await` inside

This design looked simple, but had several fundamental problems.

1. **`await` under a lock**
   A write lock was held during operations that could:

    * wait for I/O
    * stall on timeouts
    * block on OS calls

   This made lock hold time unpredictable and significantly complicated reasoning about correctness.

2. **Contention between heartbeat and user commands**
   The heartbeat ran in a separate `tokio::spawn` task and competed with API calls for the same locks, which led to:

    * implicit races
    * unstable state interleavings
    * bugs that were hard to reproduce under load

3. **Lack of strict invariants**
   There was no formal contract guaranteeing that:

    * every request will eventually complete
    * batch operations cannot get stuck
    * the system degrades correctly on partial failures, for example loss of a background task

4. **Difficulty of extension and testing**
   State management logic, concurrent access, and heavy operations were mixed together, making it harder to:

    * modify behavior
    * add new scenarios
    * write reliable tests

## Decision

The Process Manager was refactored into an **actor-based architecture** with clear separation of responsibilities.

### 1. Actor as the Single Owner of State

* All state (`processes`, `state`, batch counters) is owned by a single actor.
* The actor processes commands **sequentially** in a single event loop.
* There are no `Mutex` or `RwLock` usages in the business logic.

### 2. Separation of Control and Execution

* The actor:

    * accepts commands
    * validates them against the current state
    * manages queues and invariants
* Heavy operations (`start/stop/restart`) are executed outside the actor via an `OpRunner`.
* Results are sent back to the actor via completion messages.

### 3. Explicit Command Contract

* All operations are modeled as `ManagerCommand`.
* Each request returns its result through a `oneshot`.
* Batch operations are strictly accounted for and **cannot be silently dropped**.

### 4. Heartbeat Integrated into the Actor Loop

* The heartbeat became a regular actor event.
* It no longer mutates state concurrently with API calls.
* This removed races between automated logic and manual commands.

### 5. Deterministic Degradation

Explicit failure rules were introduced:

* If a completion channel is closed, the actor:

    * transitions to the terminal `Stopped` state
    * forcefully cancels all pending and in-flight operations
    * guarantees that no `oneshot` will hang
* The actor terminates when:

    * the command channel is closed
    * all operations are fully drained

## Consequences and Benefits

### Positive

* Predictable concurrency model with a single state owner.
* No `await` under locks in business logic.
* Clear invariants and API contracts.
* No unbounded or indefinite waits on failures.
* Improved testability and extensibility.

### Negative

* Higher architectural complexity compared to straightforward lock-based code.
* Introduction of additional concepts (actor, runner, commands).
* Higher entry barrier for understanding the code.

These costs are considered acceptable because the component is designed for long-running operation, scalability, and production use under load.

## Considered Alternatives

* Improving the existing lock-based design by limiting lock hold time or adding more mutexes.
  Rejected, as it does not eliminate the fundamental issues of concurrency and reasoning complexity.

* Using an off-the-shelf actor framework.
  Rejected to retain full control over the execution model and minimize dependencies.

## Summary

The refactor was not done for stylistic reasons or Rust “idiomaticity”, but to move to an architecture with explicit guarantees:

* a single owner of state
* no hidden races
* deterministic behavior on failures
* guaranteed completion of requests
