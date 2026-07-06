# Source Matrix Audit

Status: living verification artifact for `final.md` section 27.

Last checked: 2026-07-06 from the macOS repository checkout for source-list
shape and newly added systems inputs. The last full URL reachability pass was
2026-07-03.

## Purpose

`final.md` uses the Source Matrix as its evidentiary backbone. This document
tracks whether those sources resolve, what each source should be used for, and
where reviewers should be careful not to over-claim. It supplements `final.md`
without replacing or editing it.

This file also carries temporal toolchain and accelerator freshness snapshots.
Those snapshots are not repo pins. They are review inputs for agents who need to
make current-version claims in performance, language-boundary, compiler,
runtime, accelerator, or close-to-metal PRs.

Source-quality order for beaterOS design decisions:

1. Primary papers, official specifications, and official vendor docs.
2. Mature OS/security references and reproducible benchmarks.
3. Company blogs for direction and product constraints.
4. Speculation only when clearly marked as speculation.

## Source Extraction

Command shape:

```sh
sed -n '/^## 27\\. Source Matrix/,/^## 28\\. Final Strategic Recommendation/p' final.md |
  rg -o 'https?://[^ )]+' |
  while read url; do
    curl -L --max-time 20 --connect-timeout 10 \
      -A 'beaterOS-source-audit/0.1' -o /dev/null -s -w '%{http_code}' "$url"
  done
```

Current result:

- 86 URLs are extracted from the current `final.md` section 27.

## 2026-07-03 Reachability Pass

Result from the then-current source list:

- 51 URLs were extracted from `final.md` section 27 at that revision.
- 44 returned HTTP 200 from the command-line reachability pass.
- 7 returned HTTP 403 to `curl` but resolved in a browser-style fetch:
  - `https://wiki.osdev.org/Main_Page`
  - `https://wiki.osdev.org/Beginner_Mistakes`
  - `https://wiki.osdev.org/Required_Knowledge`
  - `https://openai.com/index/introducing-operator/`
  - `https://openai.com/index/computer-using-agent/`
  - `https://openai.com/index/chatgpt-agent-system-card/`
  - `https://www.intel.com/content/www/us/en/developer/tools/trust-domain-extensions/overview.html`

Interpretation: no `final.md` section 27 URL was found dead in the 2026-07-03
pass, but HTTP reachability is not the same as scholarly endorsement, benchmark
quality, or implementation correctness. New sources added after that pass must
be audited independently before their reachability is treated as checked.

## Issue #16 arXiv Audit

Issue #16 flagged five structurally unusual 2026 arXiv IDs. Each resolved to the
named paper on arXiv as of this audit.

| final.md entry | Verified source metadata | beaterOS use | Caveat |
| --- | --- | --- | --- |
| Agent Operating Systems (AOS): Integrating Agentic Control Planes into, and Beyond, Traditional Operating Systems, `2606.01508` | arXiv page exists; submitted 2026-06-01; title matches; subjects include cs.CR and cs.AI | Agent-control-plane framing, AOS responsibilities, OS abstraction gaps | Treat as a recent preprint, not settled architecture |
| Qualixar OS: A Universal Operating System for AI Agent Orchestration, `2604.06392` | arXiv page exists; submitted 2026-04-07; title matches; subjects include cs.AI, cs.MA, cs.SE | Multi-agent orchestration, routing, compatibility, dashboard/product surface | Application-layer orchestration, not a kernel trust model |
| Toward Securing AI Agents Like Operating Systems, `2605.14932` | arXiv page exists; submitted 2026-05-14; title matches; subject cs.CR | Security analogy between agents and OSs; privilege separation; resource mediation | Under-submission preprint; verify claims against beaterOS threat model |
| CaMeLs Can Use Computers Too: System-level Security for Computer Use Agents, `2601.09923` | arXiv page exists; submitted 2026-01-14; revised 2026-06-04; title matches | CUA isolation, planning/execution separation, prompt-injection resistance | Security/utility tradeoffs need scenario tests before adoption |
| OSWorld2.0: Benchmarking Computer Use Agents on Long-Horizon Real-World Tasks, `2606.29537` | arXiv page exists; submitted 2026-06-28; title matches | Long-horizon computer-use benchmark pressure: hidden state, many tool calls, verification debt | Benchmark results must not become product requirements without local replication |

Conclusion for issue #16: the flagged IDs are valid and should stay in the
Source Matrix. They should be marked as recent research inputs, not canonical
requirements.

## Design Implications

- Agent OS papers justify an agent control plane, but beaterOS should avoid
  replacing kernel mechanisms before it has a safe local runtime and sandbox lane.
- CUA and browser-use sources reinforce the need for manifest-before-execution,
  action-bound approvals, trusted sandbox resolution, and screenshots/DOM/a11y
  receipts.
- OSWorld-style benchmarks show that long-horizon agents fail on hidden state,
  verification, cross-source reasoning, and repeated tool calls; release gates
  need trace-property checks rather than final-answer-only scores.
- OS architecture sources reinforce minimal TCB, explicit interfaces,
  capability-style authority, and measured close-to-metal performance.
- Protocol and vendor sources are useful for compatibility direction, but
  authority must still be enforced outside model output and outside tool claims.
- Crypto, TEE, stablecoin, and post-quantum sources are input material for
  threat-model and schema agility; they are not reasons to invent cryptography.

## 2026 Systems Optimization Inputs

These sources inform the metal-touching roadmap added to `final.md` §8.1.1 and
§8.1.2. They are design inputs, not implementation commitments.

| Source | beaterOS use | Caveat |
| --- | --- | --- |
| Linux `sched_ext` documentation | Linux add-on experiments for policy-aware scheduling before native scheduler work | Linux-specific; not portable policy truth |
| Linux `io_uring` and zero-copy receive documentation | Ring-buffer IO, batching, completion records, and copy-budget discipline | API is Linux-specific and security-sensitive |
| eBPF/XDP documentation | Early packet filtering, tracing, observability, and enforcement hooks on Linux | Requires verifier limits and platform abstraction |
| Rust-for-Linux documentation | Memory-safe kernel-adjacent implementation path | Kernel Rust APIs remain evolving |
| DPDK poll-mode driver documentation | When user-space polling and direct descriptors beat interrupt-heavy paths | Burns CPU and weakens isolation if used casually |
| SPDK documentation | Zero-copy, asynchronous, lockless storage path inspiration | Specialized storage workloads only |
| Firecracker documentation | Minimal device model and microVM isolation for risky agent lanes | Depends on KVM/Linux host primitives |
| seL4 documentation | Capability microkernel and verification model for high-assurance future appliances | Broad hardware/device support is not free |
| CHERI architecture material | Hardware memory capabilities and compartmentalization direction | Research/availability constraints remain |
| Linux CXL memory-tiering background | Memory hierarchy planning for hot context, cold provenance, trace archives, and far-memory experiments | Linux- and hardware-dependent; not a near-term requirement |

## Accelerator And Custom-Silicon Inputs

These sources inform `final.md` accelerator-fabric language. They are inputs for
portable contracts, not reasons to bind beaterOS to one vendor SDK.

| Source | beaterOS use | Caveat |
| --- | --- | --- |
| NVIDIA CUDA Programming Guide and CUDA Toolkit pages: https://docs.nvidia.com/cuda/cuda-programming-guide/index.html and https://developer.nvidia.com/cuda/toolkit | GPU execution model, streams, memory movement, launch overhead, kernel optimization vocabulary, and CUDA 13.x feature tracking | CUDA-specific; keep OS contract vendor-neutral |
| NVIDIA MIG User Guide: https://docs.nvidia.com/datacenter/tesla/mig-user-guide/latest/index.html | Hardware GPU partitioning and tenant isolation model for accelerator scheduling | Only supported on specific NVIDIA datacenter GPUs |
| OpenXLA, StableHLO, and JAX docs: https://openxla.org/ and https://openxla.org/stablehlo/spec and https://docs.jax.dev/ | Portable accelerator compiler shape, StableHLO as an ML compiler interchange layer, and CPU/GPU/TPU array execution vocabulary | Framework/compiler behavior changes; conformance must be local |
| Google Cloud TPU documentation and TPU architecture docs: https://docs.cloud.google.com/tpu/docs and https://docs.cloud.google.com/tpu/docs/system-architecture-tpu-vm | TPU as custom ASIC/pod resource for matrix-heavy ML workloads through VM/GKE/Vertex surfaces | Cloud/provider-specific; APIs and generations change |
| Groq LPU architecture documentation: https://groq.com/lpu-architecture | Deterministic token-generation silicon and low-jitter inference as a distinct accelerator class | Vendor-specific claims need benchmarked local validation |
| Apple Metal and Core AI documentation: https://developer.apple.com/metal/whats-new/ and https://developer.apple.com/videos/play/wwdc2026/324/ | Local GPU, neural-accelerator, tensor, and on-device model deployment direction for macOS and Apple Silicon | Public low-level access varies by framework and entitlement |

## Language And Optimization Toolchain Inputs

These sources support `docs/optimization-agent-playbook.md` and the language
freshness notes in `docs/sota-systems-engineering.md`. They are not repo pins;
they tell agents where to verify current facts before making toolchain claims.
All versions in this table were checked against the linked official sources on
2026-07-06; re-check those sources before treating any row as a
current-version claim.

## Toolchain Freshness Ledger

Use this table when a PR says "latest", "current", "new compiler", "new
runtime", "GPU optimized", "TPU optimized", "LPU optimized", "Apple Silicon
optimized", or similar. The `Repo baseline` column says what beaterOS actually
builds or tests against today; the `Upstream version/status` column is only a
dated source snapshot.

| Component | Upstream version/status | Repo baseline | Primary source | Source type | Source date | Verified on | Optimization relevance | Claim boundary |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Rust | Rust 1.96.1 | `rust-toolchain.toml` pins 1.93.1; `Cargo.toml` declares `rust-version = "1.93"` | https://blog.rust-lang.org/2026/06/30/Rust-1.96.1/ | Official release blog | 2026-06-30 | 2026-07-06 | Compiler, Cargo, stdlib, MIR/LLVM backend behavior, safety fixes | New Rust use is not automatic; benchmark and compatibility evidence required before changing repo baseline |
| LLVM | LLVM 22.1.8 | Indirect through Rust/Apple/vendor toolchains unless explicitly invoked | https://llvm.org/ | Official project release page | 2026-06-16 | 2026-07-06 | Backend, sanitizer, C/C++ interop, vectorization, target support | LLVM version alone does not prove Rust, Apple Clang, CUDA, or vendor compiler behavior |
| Zig | 0.16.0 release; 0.17.0-dev snapshots visible on download page | No beaterOS TCB baseline | https://ziglang.org/download/ | Official download page | 2026-07-05 snapshot observed | 2026-07-06 | Freestanding and cross-compilation probes | Experimental only until toolchain stability and reviewer depth are proven |
| Swift | Swift 6.3.3 | No authority-path baseline; Apple-native/platform integration only | https://forums.swift.org/t/announcing-swift-6-3-3/87888 | Official project forum announcement | 2026-06-30 | 2026-07-06 | Apple platform APIs, UI/platform integration, possible embedded/platform experiments | Swift is not the policy, journal, receipt, or scheduler authority boundary |
| Go | Go 1.26.4 artifacts on download page | No beaterOS authority baseline | https://go.dev/dl/ | Official download page | Dynamic release page | 2026-07-06 | Non-TCB infrastructure daemon/tooling checks | Not for policy, journals, receipts, or scheduler authority |
| Python | Python 3.14.6 | Host `python3` for bounded scripts and local gates | https://www.python.org/downloads/ | Official download page | 2026-06-10 | 2026-07-06 | Audit, validation, research, replay scripts | Python scripts must remain bounded and non-authoritative |
| CUDA Toolkit | CUDA Toolkit 13.3.1 shown in CUDA archive/download surfaces | No committed CUDA backend; Linux CUDA lane is experimental readiness metadata | https://developer.nvidia.com/cuda-toolkit-archive | Official vendor archive | 2026-06 | 2026-07-06 | GPU kernels, streams, launch overhead, occupancy, memory movement, profiler compatibility | CUDA is a backend behind beaterOS admission/telemetry/receipt contracts, not the OS contract |
| NVIDIA MIG | Latest MIG guide | No committed MIG backend; partitioning input for future GPU lanes | https://docs.nvidia.com/datacenter/tesla/mig-user-guide/latest/index.html | Official vendor documentation | Dynamic latest page | 2026-07-06 | Hardware GPU partitioning and tenant isolation | Only certain NVIDIA datacenter GPUs support it; fallback isolation required |
| Apple Metal | Metal 4 direction on Apple Developer "What's new" page | Apple Silicon readiness lane is metadata only; no committed Metal backend | https://developer.apple.com/metal/whats-new/ | Official vendor documentation | Dynamic 2026 page | 2026-07-06 | Local GPU, tensors, quantization, MPS/Core ML adjacent placement, Apple Silicon profiling | Framework placement can be opaque; record limitations and CPU fallback |
| Cloud TPU | Cloud TPU docs and TPU7x/Ironwood architecture pages | No committed TPU backend | https://docs.cloud.google.com/tpu/docs | Official cloud/provider documentation | Dynamic documentation | 2026-07-06 | Matrix-heavy ML accelerator scheduling, pod/VM/GKE provider constraints | Provider-specific; admission, spend, telemetry, and receipts remain beaterOS-owned |
| Groq LPU | LPU architecture documentation | No committed LPU backend | https://groq.com/lpu-architecture | Vendor architecture documentation | Dynamic vendor page | 2026-07-06 | Deterministic low-jitter inference silicon as a distinct accelerator class | Vendor claims require measured validation and fallback |
| OpenXLA/StableHLO/JAX | Portable compiler/interchange and array execution docs | No committed XLA backend | https://openxla.org/ and https://openxla.org/stablehlo/spec and https://docs.jax.dev/ | Official project documentation | Dynamic documentation | 2026-07-06 | TPU/GPU compiler portability, graph lowering, backend placement vocabulary | Local conformance and backend telemetry required before claims |

| Source | beaterOS use | Caveat |
| --- | --- | --- |
| Rust release blog, Rust 1.96.1, 2026-06-30: https://blog.rust-lang.org/2026/06/30/Rust-1.96.1/ | Current Rust release verification and Cargo/rustup update provenance | The workspace baseline is pinned separately in `rust-toolchain.toml`; use the newer release only when a PR explicitly justifies and measures the toolchain change |
| LLVM project home/release page, LLVM 22.1.8, 2026-06-16: https://llvm.org/ | Compiler/backend, sanitizer, C/C++/Rust backend, and toolchain-version checks | LLVM version alone does not prove a Rust, Apple Clang, or vendor compiler behavior |
| Zig download page, 0.16.0 release and 0.17.0-dev snapshots: https://ziglang.org/download/ | Freestanding/cross-compilation experiment tracking | Zig remains non-TCB until stability and reviewer coverage are proven |
| Swift.org macOS install page and Swift 6.3.3 announcement: https://swift.org/install/macos/ and https://forums.swift.org/t/announcing-swift-6-3-3/87888 | Apple-native platform integration and Swift build-tooling awareness | Swift is not the beaterOS authority boundary |
| Go downloads page, Go 1.26.4 artifacts: https://go.dev/dl/ | Non-TCB infrastructure daemon/tooling version checks | Go is not used for policy, journals, receipts, or scheduler authority paths |
| Python downloads page, Python 3.14.6, 2026-06-10: https://www.python.org/downloads/ | Audit/research script runtime freshness | Python scripts must remain bounded and non-authoritative |
| NVIDIA CUDA Programming Guide: https://docs.nvidia.com/cuda/cuda-programming-guide/index.html | GPU programming model, memory hierarchy, streams, launch/occupancy vocabulary | CUDA is a backend, not the OS contract |

## Eval Statistics Inputs

These sources support `final.md` §14.9 and
`docs/design/eval-statistical-method.md`. They define how beaterOS release gates
avoid single-run point estimates for probabilistic agents.

| Source | beaterOS use | Caveat |
| --- | --- | --- |
| tau-bench, `arXiv:2406.12045`, submitted 2024-06-17: https://arxiv.org/abs/2406.12045 | pass^k reliability metric for repeated agent trials; motivates reporting reliability across k consecutive successes | Benchmark domains are not beaterOS requirements; the metric is the reusable input |
| Adding Error Bars to Evals, `arXiv:2411.00640`, submitted 2024-11-01: https://arxiv.org/abs/2411.00640 | Error bars, paired model comparisons, and experiment-planning discipline for language-model evals | Preprint guidance; beaterOS still needs local gate calibration |
| Sequential Testing for Early Stopping of Online Experiments, SIGIR 2015: https://dl.acm.org/doi/10.1145/2766462.2767729 | Sequential stopping pattern so expensive multi-trial agent evals can stop once evidence is decisive | Online-experiment setting, not agent-specific; gate configs must declare stopping rules before runs |

## Maintenance Rules

When adding a source:

- Record title, URL, source type, date checked, and why it matters.
- Prefer official links over secondary commentary.
- For arXiv papers, verify that the ID resolves to the stated title and record
  submission or revision date.
- Mark preprints, marketing posts, and speculative sources explicitly.
- Do not cite a source as evidence for claims it does not make.
- Add or update a doc-health check when citations become part of a release gate.
