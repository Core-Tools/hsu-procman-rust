# Perf tests for Process Management 
## `perf_procman_v1_vs_v2.rs`

**Core question:**

> Can the system stay responsive to queries while a long, blocking process operation is in progress?

### What the test does

* Starts multiple managed processes.
* Intentionally triggers a **long-running `stop` operation** on one process (using a gate so it blocks on purpose).
* While that stop is in progress:

    * Repeatedly queries process state (`get_all_process_info`)
    * Measures query latency and timeouts.
* Runs the exact same scenario for procman-v1 and procman-v2.

### What it demonstrates

* **procman-v1:**
  A long `stop` holds a global write lock across `await`, so *all* queries block. Query latency spikes or times out.
* **procman-v2:**
  The stop runs out-of-band in an operation runner. The control plane stays responsive, and queries complete quickly (often using cached state).

### Architectural takeaway

This test proves **control-plane isolation**.
v2 can stay responsive under slow lifecycle operations. v1 fundamentally cannot.

---

## `perf_concurrent_ops_isolation.rs`

**Core question:**

> Do heavy lifecycle operations on some processes interfere with unrelated operations on others?

### What the test does

* Runs continuous **stop+start churn** on a subset of processes.
* At the same time, repeatedly queries global process state.
* Measures query latency while churn is happening.
* Same workload shape for v1 and v2.

### What it demonstrates

* **procman-v1:**
  Churn operations block shared state. Even processes not involved in churn suffer increased query latency.
* **procman-v2:**
  Churn is isolated to the affected processes. Queries for other processes remain fast.

### Architectural takeaway

This test shows **lack of isolation in v1** vs **per-operation isolation in v2**.
v2 prevents “noisy neighbor” effects inside the manager.

---

## `perf_hot_process_isolation.rs`

**Core question:**

> Can one misbehaving or slow process degrade the entire system?

### What the test does

* Designates one process as “hot”:

    * It undergoes repeated stop+start cycles or long operations.
* Other processes remain “cold”:

    * No lifecycle changes.
* Queries global state while the hot process is under stress.

### What it demonstrates

* **procman-v1:**
  Activity on one process impacts queries for all processes because state is globally locked.
* **procman-v2:**
  Only the hot process is affected. Queries for cold processes remain responsive.

### Architectural takeaway

This test proves **process-level isolation**.
v2 scales better as the number of managed processes grows.

---

## `perf_read_heavy_write_heavy.rs`

**Core question:**

> How does the system behave under mixed read-heavy and write-heavy workloads?

### What the test does

* Generates a **read-heavy workload**:

    * Frequent state queries.
* Simultaneously generates a **write-heavy workload**:

    * Stop+start cycles on processes.
* Measures read latency under write pressure.

### What it demonstrates

* **procman-v1:**
  Writes dominate. Reads queue behind writes and suffer large latency spikes.
* **procman-v2:**
  Reads remain fast even while writes are ongoing.

### Architectural takeaway

This test highlights **read/write contention**.
v2 separates reads from writes architecturally. v1 serializes them.

---

## `perf_cancellation_semantics.rs`

**Core question:**

> If a long operation is cancelled, does the system recover immediately or stay wedged?

### What the test does

* Starts a long-running `stop` operation.
* Cancels it mid-flight.
* Immediately probes the system with strict-latency queries (before any warmup).
* Then runs a normal measurement window.

### What it demonstrates

* **procman-v1:**
  Cancellation may leave the system temporarily unresponsive due to lock state or incomplete cleanup.
* **procman-v2:**
  Cancellation is handled cleanly. The actor remains responsive immediately after abort.

### Architectural takeaway

This test proves **cancellation safety**.
v2 guarantees forward progress even when operations are aborted.

---

## Big-picture summary (for humans)

All these tests are variations of the same underlying question:

> **Does slow or failing work block the control plane?**

* procman-v1 answers: *yes*
  because it holds shared state locks across async waits.
* procman-v2 answers: *no*
  because it uses an actor model with out-of-band execution and bounded queues.

The tests do not “cheat” to make v2 look good. They simply apply pressure that real systems experience and observe what breaks.
