# GPU-Accelerated LLVM Compilation — Research Parking Lot

*Status: Hypothesis. Not validated. Return here when Koval is stable.*

---

## The Core Observation

LLVM IR is a graph. Specifically two overlapping graphs:
- **CFG** (Control Flow Graph) — basic blocks as nodes, jumps as edges
- **SSA/DFG** (Data Flow Graph) — instructions as nodes, def-use chains as edges

Classic LLVM optimization passes are graph algorithms:
- Register allocation → graph coloring (interference graph)
- Dead code elimination → reachability
- GVN → value numbering on DFG
- Instruction scheduling → topological sort on DAG
- Dominator tree construction → graph traversal

GPU accelerates graph algorithms. The question is whether this intersection is exploitable.

---

## Why the Naive Version Doesn't Work

Single function CFG = hundreds of nodes. GPU dispatch overhead dominates.
Graph structure is irregular → poor memory locality → GPU's weakness.
LLVM passes are sequentially dependent on each other — can't pipeline them.

**This is why nobody has shipped it.**

---

## Where It Might Actually Work

**The batch hypothesis:**

A large Rust project has thousands of functions. Each function has its own CFG.
Register allocation on each function is **independent** of all others.

Instead of: GPU doing graph coloring on one 200-node graph.
Instead: GPU doing graph coloring on **50,000 graphs simultaneously**, each 200 nodes.

This is embarrassingly parallel — exactly the GPU sweet spot.

Same logic applies to: per-function DCE, per-function instruction scheduling,
per-function SSA construction.

**The key reframe: don't parallelize passes, parallelize functions across passes.**

---

## What Needs to Be True for This to Work

- [ ] LLVM's internal data structures would need to be GPU-friendly (currently: mutable CPU-side state, pointer-heavy, not batchable)
- [ ] Passes would need to operate on batched function representations simultaneously
- [ ] The batch size needs to be large enough to amortize GPU dispatch overhead
- [ ] Memory transfer cost (CPU→GPU for IR, GPU→CPU for optimized IR) must be less than optimization time saved

---

## Research Questions to Investigate

**Empirical (Gemini Deep Research):**
- Does any academic work batch LLVM passes across functions for GPU execution?
- What is the actual distribution of CFG sizes in large Rust codebases? (Measure on Koval itself)
- Is there existing GPU graph coloring work that could be adapted for register allocation?
- What does the 2025 paper on GPU-native compilation actually claim vs. demonstrate?

**Theoretical (Claude falsification):**
- At what batch size does GPU amortize dispatch overhead for graph coloring?
- What is the memory footprint of batched LLVM IR — does it fit in VRAM?
- Does irregular CFG structure make GPU graph algorithms degrade to CPU-speed anyway?

**The falsifying question to ask Gemini:**
*"When does GPU graph algorithm performance degrade to match or underperform CPU for small, irregular graphs? What are the threshold conditions?"*

---

## Connection to Koval

If the batch hypothesis holds, Koval is a natural place to test it:

- Koval already controls the full compilation pipeline
- Koval knows the target hardware (GPU capabilities from `hardware.json`)
- Koval compiles many projects → large function corpus → natural batch
- GPU is already profiled by `koval-probe` (VRAM, compute capability)

A future `koval.toml` flag:
```toml
[experimental]
gpu_accelerated_codegen = true  # requires gpu.compute_capability >= X
```

---

## TAR Research Protocol for This Topic

Follow the Triangulated AI Research framework:

**Round 1 — Claude pre-screen (done)**
Model built. Key falsification point identified: irregular small graphs may negate GPU advantage regardless of batch size.

**Round 2 — Gemini Deep Research (todo)**
Query: *"GPU-accelerated compiler optimization passes batch processing graph coloring register allocation"*
Falsification query: *"failure cases GPU graph algorithms small irregular graphs overhead dominates"*

**Round 3 — Claude reconciliation (todo)**
Do the papers found actually address the batch hypothesis or only single-function parallelism?
Does the empirical data on CFG size distributions support or undermine viability?

**Decision gate:**
If after Round 3 there is no primary research showing batch GPU compilation with measured speedup on real codebases — treat as open research problem, not engineering task.

---

## What Would "Validated" Look Like

A paper or project showing:
1. Batch GPU execution of at least one LLVM pass (any pass)
2. On real compiler workloads (not synthetic graphs)
3. With measured speedup over parallel CPU baseline (not just sequential CPU)
4. With CFG sizes representative of real programs

Until all four boxes are checked — this remains a hypothesis worth watching, not building.

---

*Come back after: Koval v1 is shipping, Koval has a corpus of real compilation jobs to measure CFG size distribution.*

---

## Scope Expansion: This Is Not Rust-Only

**C and C++ compile to LLVM IR via clang.** Same IR, same passes, same graph structure.

**The Linux kernel compiles with clang/LLVM today** — this is production, not experimental.
ClangBuiltLinux project has been stable for years. Android has compiled its kernel
via clang since 2020. Google and Meta run this in production.

```bash
make CC=clang LLVM=1  # full kernel as LLVM IR, right now
```

The only parts of the kernel that don't pass through LLVM IR are inline assembly blocks
— everything else is representable.

**What this means for the batch hypothesis:**

The Linux kernel is ~30 million lines of C. Compiled via clang it produces an enormous
function corpus — orders of magnitude larger than any single Rust project. If batch size
is the key variable for GPU amortization, the kernel is the most favorable workload
imaginable.

**What this means for Koval's ceiling:**

Koval built as a Rust-specific tool is one thing. Koval built as an
**LLVM IR pipeline with GPU-accelerated passes** is language-agnostic — it works for
any language with a clang or rustc front-end. That includes C, C++, Rust, and anything
else that emits LLVM IR (Zig, Swift, Julia in some configurations).

This is a different project with a different scope. Note it, don't build it yet.

**Additional research question to add to Round 2:**
*"What is the function count distribution in clang-compiled Linux kernel builds?
What is the total LLVM IR size in bytes for a full kernel compilation?"*

**The strategic implication:**
If GPU-accelerated LLVM passes are ever validated, the right architecture is a
**language-agnostic LLVM IR processing layer** — not a Rust-specific build tool.
Koval would either evolve into that or feed into it. Either way, Koval's
`hardware.json` and probe infrastructure remains directly reusable.
