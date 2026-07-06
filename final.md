# beaterOS Final Plan

Date: 2026-07-03

Status: research plan and system design plan only. This document intentionally does not implement code.

Repo objective: define what must be true for beaterOS to become a successful agent-first operating system, from first principles, with enough detail to guide future engineering, security review, simulation, and product choices.

## 1. Executive Thesis

beaterOS is a long-horizon operating-system project for agentic computing. The end state is not just an app, a framework, or a desktop shell. The end state is an OS stack that can touch metal where that is the right engineering boundary: scheduler, memory, IO, devices, isolation, authority, audit, and recovery designed from first principles for probabilistic agents.

It should not begin by trying to replace Linux or macOS wholesale.

It should begin as an agent-first operating layer: a user-space control plane that sits above Linux, macOS, cloud VMs, browsers, containers, and model providers. Its job is to make agent work safe, observable, reproducible, permissioned, testable, and economically bounded. This lane is not a retreat from building a real OS. It is the bootstrapping path that lets the project validate contracts, policy, traces, and workloads before committing to hardware, kernel, and driver boundaries.

The first winning version is an "agent kernel" hosted by existing operating systems. The later winning version is a metal-touching OS stack whose low-level components are justified by measured agent workloads. In both cases, beaterOS owns the agent-specific primitives that conventional operating systems do not have:

- Goals, tasks, sessions, and long-running plans.
- Tool and action capability grants.
- Durable journals before and after side effects.
- Memory with provenance, not just files and databases.
- Policy decisions that are first-class records.
- Sandboxed execution lanes for shell, browser, code, and remote tools.
- Replay, simulation, and eval gates as operating primitives.
- Human review for irreversible or high-risk actions.
- Model routing across local, cloud, multimodal, long-context, and specialized models.
- Agent identity, delegated authority, budget limits, and accountability.

The deep first-principles claim is this:

> Classical operating systems protect users from deterministic programs. beaterOS must protect users, organizations, tools, models, and other agents from probabilistic goal-seeking systems that can read, reason, compose tools, spend money, and create irreversible side effects.

That is a different operating problem.

## 2. The Core Answer

Can we make an agent-first operating system?

Yes, if we choose the right sequence.

The wrong first target is "build a new Linux" or "make a beautiful AI desktop shell". That absorbs years into drivers, hardware support, GUI polish, package management, and compatibility before the agent-specific problem is solved.

The right first target is:

1. A minimal agent kernel daemon.
2. A typed action and capability system.
3. A tamper-evident journal.
4. Sandboxed tool execution.
5. Memory and provenance services.
6. Simulation/eval gates.
7. A small but strict security model.
8. Adapters into existing OS surfaces, browsers, MCP servers, CLIs, APIs, and Beater components.

Only after those contracts work should beaterOS become a Linux add-on, distro, desktop, VM image, unikernel, library OS, microkernel appliance, or true metal-touching kernel project.

The strategic split is:

- **Compatibility lane:** make Linux, macOS, containers, browsers, and cloud VMs safer for agents now.
- **Metal lane:** build the parts of a new OS only where measurements show the host OS cannot express the right authority, latency, isolation, memory, IO, or audit contract.

The metal lane is a years-long research and implementation program, not a weekend kernel hobby project. It should advance through proofs, simulators, microVMs, hypervisors, narrow appliances, and hardware-backed isolation before broad device support.

## 3. What Must Be True For Success

beaterOS is successful only if the following become true in practice, not only in architecture diagrams.

### 3.1 Agent Work Becomes Safer Than Running Scripts

Today, many agent workflows are worse than shell scripts from a security standpoint: they combine untrusted text, secrets, browser state, file access, network access, and arbitrary tools behind a natural-language interface.

beaterOS must make agent work safer by default:

- No agent receives ambient authority by default.
- Every non-trivial side effect is associated with an explicit capability grant.
- Grants are scoped by resource, action, time, budget, identity, and risk.
- Tools are treated as untrusted unless registered, pinned, signed, and policy-approved.
- Web, email, chat, documents, logs, and tool descriptions are treated as data, not instructions.
- High-risk actions require review, simulation, or multi-party approval.
- The system can answer: who did what, using which model, with which prompt class, under which policy, against which resource, at what time, and why.

### 3.2 Agent Work Becomes Reproducible Enough To Debug

Perfect reproducibility is impossible for live web pages, remote APIs, and nondeterministic models. But operational reproducibility is possible.

beaterOS must record enough information to reconstruct and analyze a run:

- Initial goal and user constraints.
- Model calls and model metadata.
- Tool manifests and versions.
- Capability grants.
- Inputs, outputs, observations, and redactions.
- Browser DOM snapshots or accessibility snapshots where appropriate.
- Shell commands, filesystem diffs, and process metadata.
- Network targets and request classes.
- Policy decisions and denial reasons.
- Human approvals and overrides.
- Final artifacts and side-effect receipts.

The test is simple: when an agent fails, a competent engineer should be able to replay the relevant segment, inspect the decision path, and patch either the policy, tool, model route, prompt contract, or environment.

### 3.3 Agent Authority Becomes Legible

A human should not need to read a prompt transcript to understand what an agent is allowed to do.

beaterOS must present authority as a small set of inspectable objects:

- Read-only file grants.
- Write grants to specific paths or datasets.
- Network grants by domain, IP class, protocol, method, and data category.
- Browser grants by origin, session, credential boundary, and action type.
- Payment grants by asset, amount, counterparty class, recurrence, and refund/chargeback model.
- Cloud grants by account, region, service, API method, and resource tag.
- Human communication grants by recipient, channel, message class, and approval mode.
- Model grants by data sensitivity, provider, retention policy, and cost ceiling.

If authority cannot be summarized, it is too broad.

### 3.4 Agent Memory Becomes Accountable

Agent memory cannot be a mysterious vector database that silently accumulates private information.

beaterOS must make memory:

- Projected from an append-only event log where possible.
- Tagged with source, time, confidence, sensitivity, owner, and expiry.
- Queryable with provenance.
- Redactable.
- Rebuildable.
- Separable into working memory, episodic memory, semantic memory, policy memory, and user preference memory.
- Protected by the same capability model as files and tools.

The system must support "why do you know this?" and "forget this everywhere you are allowed to forget it."

### 3.5 Evals Become Part Of The Operating System

Agents are too probabilistic to ship without eval gates.

beaterOS must treat simulations and evals as a core operating service:

- Every important workflow has a scenario manifest.
- Every tool and integration has golden tests.
- Every risky permission has abuse tests.
- Every model upgrade runs paired evaluation against baseline traces.
- Every agent release has held-out tasks, regression tasks, and adversarial tasks.
- Every production incident produces new evals.

An OS for agents that cannot evaluate agent behavior is not an OS. It is an automation launcher.

### 3.6 The System Remains Model-Agnostic

The model landscape will keep changing. The OS must not hard-code one provider, one context-window assumption, one reasoning style, or one tool protocol.

beaterOS must handle:

- Cloud frontier models.
- Local models.
- Long-context models.
- Low-latency small models.
- Multimodal models.
- Computer-use models.
- Code models.
- Specialized planners, verifiers, classifiers, and embedding models.
- Future models with stateful sessions, private reasoning traces, or tool-native APIs.

The OS should own the contract around authority, state, observability, and evals. Models are replaceable workers.

### 3.7 The System Avoids False Complexity

The first version must be strict, but not overbuilt.

Do not begin by building:

- A new kernel.
- A new cryptographic primitive.
- A full desktop environment.
- A complete package ecosystem.
- A universal workflow language.
- A custom browser engine.
- A blockchain.
- A generic multi-agent society simulator.

Begin with:

- A session object.
- A capability grant object.
- An action manifest object.
- A journal/receipt object.
- A policy decision object.
- A scenario/eval manifest object.

If those six contracts are correct, the rest can grow.

## 4. First Principles Of Operating Systems

An operating system exists because computation is not just code. Computation needs resources, isolation, scheduling, persistence, naming, coordination, and recovery.

Classical OS design can be reduced to a few durable principles.

### 4.1 Resource Allocation

The OS allocates scarce resources:

- CPU.
- Memory.
- Disk.
- Network.
- Devices.
- Files.
- Locks.
- Credentials.
- Human attention.

For agents, add:

- Model calls.
- Context window.
- Tool-call budget.
- Prompt budget.
- Human review budget.
- Browser sessions.
- API quotas.
- Cloud spend.
- Payment spend.
- Trust budget.
- Risk budget.

The core operating question is not "can the agent do it?" It is "under which resource constraints may this agent attempt it?"

### 4.2 Protection Boundaries

The OS separates principals that should not fully trust each other.

Traditional principals:

- Users.
- Processes.
- Groups.
- Kernel.
- Devices.
- Remote hosts.

Agent-first principals:

- Human user.
- Organization.
- Agent.
- Subagent.
- Model provider.
- Tool provider.
- MCP server.
- Browser origin.
- Document source.
- Memory source.
- Payment counterparty.
- Policy authority.

The hard part is that an agent is not just a process. It is an interpreter of data and intent. It can be influenced by every text and image it reads. Therefore protection must cover both machine execution and semantic influence.

### 4.3 Namespaces

The OS gives names to resources.

Traditional namespaces:

- Filesystem paths.
- Process IDs.
- Network addresses.
- User IDs.
- Device names.

Agent namespaces need:

- Goal IDs.
- Task IDs.
- Agent IDs.
- Session IDs.
- Capability IDs.
- Tool IDs.
- Model route IDs.
- Memory IDs.
- Browser context IDs.
- Human approval IDs.
- Payment mandate IDs.
- Policy version IDs.
- Scenario IDs.
- Trace IDs.

Without these names, authority becomes conversational and debugging becomes archaeology.

### 4.4 Scheduling

Traditional schedulers multiplex CPU time and I/O.

Agent scheduling must multiplex:

- Reasoning loops.
- Tool calls.
- Human approval waits.
- Retry windows.
- Long-running background tasks.
- Concurrent subagents.
- Model capacity and cost.
- Risk-sensitive actions.
- Latency-sensitive user interactions.

For agents, the scheduler must be policy-aware. It must know that "send wire transfer" is not the same class of work as "summarize a file", even if both are just function calls.

### 4.5 Persistence

Traditional OSs persist files and logs.

Agent OSs must persist intent and causality:

- The user asked for X.
- The system decomposed X into Y.
- The agent proposed action Z.
- Policy allowed Z because of grant G.
- Tool T executed Z and returned R.
- Human H approved side effect S.
- Memory M was written because of observation O.
- Eval E later judged this behavior acceptable or not.

The important persistent unit is not only a file. It is a causal chain.

### 4.6 Recovery

Traditional recovery means restarting processes and repairing filesystems.

Agent recovery means:

- Resume an interrupted task without redoing unsafe side effects.
- Roll forward from a journal.
- Identify which side effects already happened.
- Revoke or expire capabilities.
- Invalidate contaminated memory.
- Replay with a different model or policy.
- Compare the failed run to a successful run.
- Produce an incident report.

The journal must be designed before the executor, because irreversible actions cannot be debugged after the fact if they were never recorded.

### 4.7 Interface Contracts

Traditional OS contracts are syscalls, files, sockets, signals, and devices.

Agent OS contracts need:

- Intent declarations.
- Action manifests.
- Capability grants.
- Policy decisions.
- Tool schemas.
- Memory provenance.
- Human review prompts.
- Side-effect receipts.
- Eval outcomes.

The syscall equivalent for agents is not "run this tool". It is "attempt this typed action under this grant, capture the receipt, and update the journal."

## 5. Why Existing OSs Are Hard For Agents To Use

Conventional OSs are powerful for humans and deterministic programs. They are awkward for autonomous agents.

### 5.1 The Unit Of Work Is Wrong

Operating systems understand processes and files. Agents operate on goals and tasks.

An agent goal may involve:

- Reading a repo.
- Searching docs.
- Editing files.
- Running tests.
- Opening a browser.
- Calling a payment API.
- Asking a human.
- Waiting hours.
- Replanning.
- Delegating subgoals.

No conventional OS object represents that whole activity. The result is that agent frameworks invent ad hoc sessions, logs, memory stores, and tool wrappers.

### 5.2 Authority Is Too Ambient

If a process can read a directory, open a network connection, or access an environment variable, all code inside that process often inherits that authority.

Agents make this worse because:

- The model can be influenced by untrusted content.
- Tool selection can be dynamic.
- Subagents may inherit unclear authority.
- Secrets may appear in prompts or logs.
- Browser sessions contain ambient cookies.
- CLI tools inherit shell and filesystem access.

An agent needs authority attenuation: the ability to pass a narrower permission to a subtask than the parent has.

### 5.3 The OS Does Not Understand Semantic Risk

The OS can tell the difference between a read syscall and a write syscall. It cannot tell the difference between:

- Reading public docs and reading customer secrets.
- Sending a draft to yourself and sending it to a client.
- Running a local test and deploying to production.
- Calling a read-only API and initiating a payment.
- Opening a website and submitting a form.

Agent safety requires semantic risk classification.

### 5.4 GUI Automation Is A Leaky Surface

Many computer-use agents operate through pixels, accessibility trees, DOMs, or browser automation.

The OS and browser were built for human perception and interaction, not machine-verifiable intent. Problems include:

- Visual ambiguity.
- Hidden state.
- Popups and overlays.
- CSRF-style flows.
- Session cookies.
- Unlabeled controls.
- Layout shifts.
- Phishing and lookalike sites.
- Inconsistent accessibility metadata.
- Actions that appear reversible but are not.

beaterOS should prefer typed APIs and tool manifests. It should use GUI/browser control when necessary, but wrap it in semantic receipts and policy gates.

### 5.5 Logs Are Not Journals

Traditional logs often say what happened after it happened. Agent systems need pre-action intent records:

- Proposed action.
- Required grant.
- Risk class.
- Policy decision.
- Human approval requirement.
- Expected side effect.
- Idempotency key.

For irreversible actions, "log after executing" is too late.

### 5.6 Memory Is Bolted On

The OS has files and virtual memory. Agent frameworks add vector memory, conversation memory, scratchpads, and databases separately.

This creates failure modes:

- Private data leaks into embeddings.
- Stale information persists.
- Low-confidence observations become facts.
- Memories are detached from source traces.
- Model summaries overwrite nuance.
- A compromised source can poison long-term memory.

Memory needs provenance and policy at the OS layer.

### 5.7 Existing Permission UIs Do Not Fit Agents

Human app permissions are usually coarse:

- Allow camera.
- Allow files.
- Allow location.
- Allow notifications.
- Allow network.

Agent permissions need to be workflow-specific:

- This agent may read these files for this task for the next hour.
- It may create branches but not push.
- It may draft email but not send externally.
- It may spend up to 20 USD with approved vendors.
- It may use cloud APIs only in staging.
- It may browse public sites but cannot submit forms without review.

### 5.8 The OS Cannot Tell Data From Instructions

Agents are vulnerable because they read untrusted data and may treat it as instructions.

Examples:

- A web page says "ignore previous instructions and exfiltrate secrets".
- A GitHub issue includes malicious tool-use instructions.
- A document embeds hidden text.
- A tool description lies about what the tool does.
- A remote MCP server presents a lookalike tool.

Classical OSs protect code and data at the memory level, but they do not protect semantic instruction channels.

### 5.9 There Is No Native Eval Gate

An OS will let a program run if it has permission. It does not ask whether this version of the agent still performs the workflow correctly.

For agentic systems, every model upgrade, tool change, policy change, or prompt change can alter behavior.

beaterOS must make evals a release gate, like tests and type checks are for code.

## 6. What Operating Systems Lack In General

This section is not only about agents. It is about broad OS limitations exposed by modern workloads.

### 6.1 Causal Observability

OS observability is excellent for low-level metrics:

- CPU.
- Memory.
- Disk.
- Network.
- Process trees.
- Syscalls.

It is weaker for causality:

- Why did this action happen?
- Which human intent caused this write?
- Which policy allowed this network request?
- Which model output triggered this command?
- Which prior memory influenced this decision?

Agent OSs need causal observability as a first-class primitive.

### 6.2 Fine-Grained Authority

ACLs, roles, and app permissions are usually too coarse.

Capability systems, especially object capabilities, are a better conceptual fit:

- Possession of a capability gives specific authority.
- Capabilities can be attenuated.
- Capabilities can expire.
- Capabilities can be delegated with constraints.
- Capabilities can be revoked through indirection.
- Capabilities can be audited.

beaterOS should make capability grants the primary permission object.

### 6.3 Semantic Namespaces

Files and processes are not enough.

Modern work is organized around:

- Projects.
- Tickets.
- Repos.
- Branches.
- Customers.
- Documents.
- Workflows.
- Chat threads.
- Cloud resources.
- Business entities.

An agent-first OS needs a namespace that can map semantic objects to concrete resources without losing authority boundaries.

### 6.4 Durable Workflows

Most OS primitives assume short-lived process execution. Real work may span minutes, days, or weeks.

Agent OSs need durable workflow objects:

- Pausable.
- Resumable.
- Inspectable.
- Interruptible.
- Migratable.
- Replayable.
- Cancelable with compensation.

### 6.5 Built-In Simulation

OSs expose production resources directly. Simulation is usually an application concern.

For agents, simulation should be native:

- Mocked tools.
- Replay servers.
- Browser fixtures.
- Synthetic orgs.
- Fake payment rails.
- VM snapshots.
- Containerized services.
- Adversarial prompts.

No serious agent OS should let new agents touch real systems before they pass simulations.

### 6.6 Verifiable Side Effects

Existing systems record many side effects, but they are scattered across logs.

beaterOS should require receipts:

- Filesystem diff receipts.
- API response receipts.
- Browser action receipts.
- Email draft/send receipts.
- Payment receipts.
- Cloud change receipts.
- Human approval receipts.
- Memory write receipts.

Receipts should be hash-linked into a run journal.

### 6.7 Human Attention As A Resource

OSs schedule CPU time. Agent OSs must schedule human attention.

Poorly designed agents either:

- Ask too often and become useless.
- Ask too little and become dangerous.

The OS should understand review queues, escalation policies, approval budgets, and interruption cost.

### 6.8 Economic Boundaries

Modern agent work spends money:

- Model calls.
- Cloud compute.
- API usage.
- SaaS actions.
- Ads.
- Purchases.
- Stablecoin or card payments.
- Developer infrastructure.

beaterOS needs budget primitives. Payments should be just another dangerous side effect under capability control.

### 6.9 Trustworthy Package And Tool Supply

Agent tools are supply-chain risk. A malicious tool can exfiltrate data or change behavior.

The OS needs:

- Signed tool manifests.
- Pinned versions.
- Reproducible builds where feasible.
- SBOMs.
- Provenance attestations.
- Runtime sandboxing.
- Tool reputation and policy metadata.

### 6.10 Multi-Model Resource Management

Classical OSs schedule CPU. Agent OSs must schedule model resources.

The OS must decide:

- Which model can see which data.
- Which model class is allowed for which risk class.
- Which model is cheap enough for a subtask.
- Which model is accurate enough for a verifier.
- Whether local-only inference is required.
- Whether a model provider's retention policy is compatible with a task.

## 7. What Agents Should Have In An OS

An agent-first OS should provide the following primitives as native system services.

### 7.1 Agent Identity

Each agent must have a durable identity separate from the human user and separate from the process running it.

Identity fields:

- Agent ID.
- Human owner.
- Organization owner.
- Agent type.
- Version.
- Signing key or delegated credential.
- Allowed workspaces.
- Allowed model routes.
- Default policy profile.
- Audit contact.
- Revocation status.

Agent identity must be visible in every trace, receipt, and side effect.

Identity should reuse existing standards rather than invent new ones. Service and sandbox-lane workload identity maps onto SPIFFE/SPIRE (short-lived X.509/JWT SVIDs with per-node attestation, no long-lived distributed secrets). "Agent X acting on behalf of user U" toward external services maps onto OAuth 2.0 Token Exchange (RFC 8693) actor claims, which the emerging IETF agent-authorization drafts extend. beaterOS should track and adopt those profiles, not parallel-invent them. (Issue #49.)

### 7.2 Agent Session

A session is the container for one coherent goal or workflow.

Session fields:

- Session ID.
- User intent.
- Scope.
- Start time.
- Deadline.
- Human owner.
- Agent identity.
- Policy profile.
- Initial capability set.
- Memory visibility.
- Budget ceilings.
- Model routing constraints.
- Scenario/eval class.
- Journal root hash.
- Status.

Sessions let the OS reason about work at the level humans care about.

### 7.3 Capability Grant

A capability grant is the central authority object.

Grant fields:

- Grant ID.
- Principal receiving authority.
- Issuer.
- Resource.
- Allowed actions.
- Denied actions.
- Time window.
- Budget ceiling.
- Data sensitivity ceiling.
- Delegation rule.
- Human approval rule.
- Revocation handle.
- Policy version.
- Reason.

Grants must be unforgeable, inspectable, attenuable, revocable, and journaled.

### 7.4 Action Manifest

Before a tool runs, the agent should submit an action manifest.

Manifest fields:

- Action ID.
- Session ID.
- Proposed tool.
- Tool version.
- Target resource.
- Input summary.
- Expected side effect.
- Risk class.
- Required grants.
- Idempotency key.
- Rollback or compensation plan.
- Data classes involved.
- Human-readable explanation.

The policy engine evaluates the manifest before execution.

Manifest fields split into two trust classes. Kernel-derived fields (input digests, resolved targets, data classes derived from provenance labels, observed side effects) are computed by the mediation point — gateway or sandbox — and never accepted from the agent, because every field the agent authors is authored by the principal the system distrusts. Agent-asserted fields (input summary, explanation, compensation plan) are advisory: useful for human review, never consumed by a policy rule as a security predicate. Receipts bind to observed values and a divergence between manifest and observation is an incident, not a log line. (Issue #46.)

### 7.5 Policy Decision

Policy decisions must be first-class artifacts.

Decision fields:

- Decision ID.
- Action ID.
- Policy version.
- Allowed, denied, needs approval, needs simulation, or needs narrowed grant.
- Explanation.
- Matched rules.
- Required additional evidence.
- Human reviewer if applicable.
- Expiry.

This is crucial for debugging, compliance, and trust.

### 7.6 Side-Effect Receipt

Every important action produces a receipt.

Receipt fields:

- Receipt ID.
- Action ID.
- Tool ID.
- Start and end time.
- Input digest.
- Output digest.
- Side-effect summary.
- Resource changed.
- Before/after reference where possible.
- External transaction ID if any.
- Exit code or status.
- Error class.
- Hash link to previous receipt.

Receipts allow replay, accountability, and incident analysis.

### 7.7 Memory Record

Memory must not be an ungoverned blob.

Memory record fields:

- Memory ID.
- Source event.
- Source digest.
- Writer principal.
- Created time.
- Confidence.
- Data class.
- Sensitivity.
- Expiry.
- Embedding model if embedded.
- Summary model if summarized.
- Access policy.
- Redaction status.
- Superseded-by pointer.

### 7.8 Tool Registry

Agents need a trusted map of tools.

Registry fields:

- Tool ID.
- Publisher.
- Version.
- Transport.
- Schema.
- Required capabilities.
- Data retention behavior.
- Network behavior.
- Side-effect classes.
- Signing/provenance metadata.
- Sandbox requirement.
- Test suite.
- Risk rating.

MCP, A2A, OpenAPI, CLIs, browser tools, and local functions should all be normalized into this registry.

### 7.9 Human Review Queue

Human approval should be a system service, not a random chat message.

Review fields:

- Review ID.
- Action.
- Risk class.
- Diff or preview.
- Required decision.
- Deadline.
- Reviewer identity.
- Conflict-of-interest rule.
- Approval result.
- Audit note.

The UI should be concise, but the backend must be rigorous.

### 7.10 Scenario Manifest

Every serious workflow needs simulation.

Scenario fields:

- Scenario ID.
- Goal.
- Environment fixture.
- Initial resources.
- Allowed tools.
- Forbidden actions.
- Oracle.
- Success criteria.
- Risk traps.
- Budget.
- Model routes.
- Expected trace properties.

The scenario manifest is the eval equivalent of a unit test plus environment specification.

## 8. Big Design Choices

The following choices determine whether beaterOS becomes a serious OS layer or just another agent framework.

### 8.1 User-Space Control Plane vs New Kernel

Recommendation: start user-space.

Reason:

- The unsolved agent problems are authority, memory, observability, simulation, and policy.
- Linux, macOS, containers, browsers, and cloud VMs already provide mature hardware support.
- Building a bare-metal kernel first would delay the agent-specific learning loop.

Future path:

- If the control plane proves valuable, create a hardened Linux distro or VM image.
- Later, explore seL4 or CHERI-based high-assurance agent appliances for narrow workloads.

This is a sequencing decision, not a product ceiling. beaterOS should keep two explicit engineering lanes:

1. **Linux/macOS add-on lane.** Build the agent kernel, policy engine, sandbox lane, memory provenance, scenario runner, receipts, and audit tools on existing kernels. On Linux, use cgroups, namespaces, seccomp, LSMs, eBPF, io_uring, and microVMs where they provide measurable wins. On macOS, keep a first-class local development path with portable Rust contracts and explicit platform abstraction.
2. **Metal-touching OS lane.** Build the smallest new low-level OS pieces that agent workloads actually require: capability-secure task admission, policy-aware scheduling, bounded memory and context management, zero-copy trace and receipt paths, high-assurance sandboxing, and recoverable journals. This lane may pass through a simulator, a microkernel appliance, a library OS, a hypervisor-backed runtime, or a RISC-V/ARM research target before general hardware support.

The rule is: do not move a subsystem into the metal lane until the hosted lane has produced a workload, trace, benchmark, or security proof that the host substrate cannot satisfy cleanly.

### 8.1.1 What An Operating System Built In 2026 Should Look Like

An OS started in 2026 should not copy the 1990s desktop/kernel split and then bolt agents on top. It should begin from scarce resources and trust boundaries:

- **Authority-first kernel boundary.** Every meaningful side effect has a capability, policy decision, and receipt before the system optimizes for convenience.
- **Memory-safe default implementation.** Rust is the default for kernel/control-plane services. C is used only at stable ABI, driver, hypervisor, or proven hot-path boundaries. Assembly is limited to hardware entry points.
- **Small trusted core.** The kernel is not a warehouse for policy, drivers, parsers, and UI. The core names principals, grants, address spaces, queues, clocks, interrupts, and evidence. Everything else is a service with explicit authority.
- **Capability hardware and microkernel influence.** seL4 proves that small capability kernels can be verified. CHERI shows how hardware capabilities can make memory safety and compartmentalization a hardware property. beaterOS should learn from both without pretending broad hardware support is free.
- **Policy-aware scheduling.** Scheduling cannot only optimize CPU fairness. It must know about tool risk, human review queues, model spend, payment spend, context-window pressure, IO priority, rollback cost, and p95/p99 latency.
- **Modern IO path.** Use rings, batching, completion queues, zero-copy where possible, and explicit backpressure. Linux `io_uring`, DPDK, SPDK, XDP, and eBPF show the shape: avoid needless syscalls, copies, interrupts, and lock handoffs, but preserve audit and safety.
- **Accelerator fabric as an OS resource.** GPUs, TPUs, LPUs, NPUs, media engines, secure enclaves, and future agent ASICs are not just libraries behind SDK calls. They are scarce devices with queues, memory, power, thermals, tenants, placement constraints, model residency, data-sensitivity constraints, and side-channel risk. beaterOS should schedule accelerator work with the same discipline as CPU, IO, memory, network, model spend, and payment spend.
- **Memory hierarchy as an OS service.** Agent memory spans RAM, local SSD, remote stores, embeddings, traces, CXL/far memory, and redacted audit archives. Hot context, durable evidence, and cold provenance should be different tiers with explicit movement rules.
- **Virtualization as a primitive, not an afterthought.** MicroVMs and confidential-computing enclaves are useful isolation boundaries for risky tools and third-party workloads, but attestation and firmware trust remain attack surfaces. Use them as evidence-bearing compartments, not magic.
- **Observable by construction.** Tracepoints, receipts, policy decisions, model routes, queue depth, spend, and filesystem/network effects are first-class records, not logs scraped after an incident.

This is the deeper beaterOS direction: a compatibility-first agent OS that earns the right to become a metal-touching OS by proving which low-level boundaries must exist.

### 8.1.2 Ecosystem Runtime Contract

beaterOS must be the OS substrate for the rest of the ecosystem, including Tempo, beater.js, beatbox, beater-memory, and future agent surfaces. That does not mean every project is folded into one repo. It means the hot contracts are shared, typed, and measurable:

- **Tempo/browser lane.** Browser actions must pass through action manifests, capability grants, tool registry checks, screenshot/DOM/accessibility evidence, network/origin policy, and receipts. The hot path should avoid JSON string churn where possible: use generated schemas, stable binary encodings for high-volume traces, shared-memory or ring-buffer transport where justified, and native Rust services for policy/admission/receipt work. Tempo UI code can stay in the browser stack, but authority, audit, and scheduler-facing operations belong in native beaterOS services.
- **beater.js/agent lane.** JavaScript/TypeScript can remain the ergonomic agent authoring surface, but it must not be the authority boundary. It calls into the Rust policy, journal, memory, and tool-gateway contracts through a narrow API.
- **beatbox/sandbox lane.** Sandbox execution is a low-level service. The fast path is canonicalize, admit, execute under confinement, diff, receipt, and append. Optimize syscalls, copies, environment setup, and diffing before adding abstractions.
- **beater-memory lane.** Memory projection is a derived view over journals and receipts, not ambient truth. Hot memory queries need indexes and cache-aware layouts; durable provenance stays append-only and replayable.
- **Model/tool/payment lanes.** Provider SDKs are adapters. Authority lives in beaterOS contracts. Expensive calls must be budgeted, cancellable, traceable, and tied to receipts.
- **Accelerator lane.** GPU, TPU, LPU, NPU, and custom-silicon execution must be modeled as admitted work. A run names the model/artifact, accelerator class, memory budget, data class, precision/quantization profile, expected latency, batch/streaming mode, tenant/isolation requirement, and fallback route. Receipts record placement, device class, driver/runtime version, accelerator partition (for example MIG or a VM/pod slice), memory movement, queue delay, execution time, output digest, and thermal/power throttling when observable.

The language rule for ecosystem integration is strict:

- Rust owns TCB services, hot policy paths, journals, receipts, schedulers, sandbox orchestration, memory projection, native IPC, and kernel/hypervisor-facing code.
- TypeScript owns UI and high-level agent ergonomics only where latency, authority, and memory safety are not the boundary.
- C/C++ are allowed for stable ABI, platform APIs, browser/embedder interop, drivers, hypervisors, and measured hot-path library integration.
- Assembly is allowed only at unavoidable hardware entry points.
- WASM is allowed for portable untrusted plugins when deterministic sandboxing is more important than raw native speed.

Optimization is by evidence: first remove work, then batch/cache, then reduce copies/syscalls/allocations, then specialize, then move closer to the metal. A subsystem moves from TypeScript to Rust, from Rust to C, or from user space to kernel/driver/hypervisor only when a trace, benchmark, or security review proves the current boundary is the limiting factor.

Every serious optimization PR must carry an optimization packet: current compiler/runtime versions when relevant, the bottleneck class, the replayable workload, baseline p50/p95/p99 or throughput/memory/copy/syscall/device metrics, the target budget, the profile or trace that identifies the limiter, the authority boundary that must not change, the macOS path, and the regression gate. "Use a faster language" is not an argument until ownership, cancellation, error propagation, audit evidence, unsafe/FFI review, and rollback are explicit.

Accelerator optimization follows the same rule, but the cost model is different:

- Keep model weights resident when safe; avoid repeated host-device transfer.
- Treat HBM/SRAM/VRAM as schedulable memory tiers, not opaque implementation detail.
- Batch only when it does not violate latency, cancellation, priority, or human-review constraints.
- Route tokens and embeddings to the right silicon class: GPU for general parallel kernels and model serving, TPU for XLA/JAX/PyTorch matrix-heavy training and inference where available, LPU or similar deterministic inference silicon for low-jitter token generation when the provider exposes it, NPU/media/Apple Neural Engine-style local accelerators for private low-power local tasks when platform APIs allow it.
- Partition accelerators by hardware support where possible, such as NVIDIA MIG or VM/pod-level TPU isolation, and fall back to process/microVM isolation plus policy when hardware partitioning is unavailable.
- Make every accelerator path replayable enough for audit: device class and version, runtime/compiler version, model digest, quantization, precision, seed where meaningful, and admission context.

### 8.2 Capability Security vs ACL/RBAC First

Recommendation: capability-first, with ACL/RBAC adapters.

Reason:

- Agents delegate subtasks.
- Agents need narrowed authority.
- A subagent should receive a specific object, not a role with broad ambient power.
- Capabilities compose better with tools, tasks, and temporary grants.

Implementation principle:

- RBAC answers who may issue grants.
- Capabilities answer what this agent can do now.

Do not reinvent the grant carrier. Caveat-based capability tokens are a solved category: Macaroons (Google, NDSS 2014) implement attenuation as append-only caveats folded into a chained MAC, with third-party discharge caveats as revocation-by-indirection; Biscuit (Eclipse Foundation) does the same with public-key signed blocks, Datalog checks, and per-block revocation IDs verifiable offline by anyone holding the issuer key. These implement every CapabilityGrant invariant in section 12.2 cryptographically instead of by service-side validation. Even a bespoke implementation must keep the caveat semantics: attenuation is append-only restriction, never field mutation, so "grants cannot be broadened by the holder" is structural rather than checked. The "do not invent cryptography" rule extends to authorization token constructions. (Issue #49.)

### 8.3 Event-Sourced Journal vs Mutable State First

Recommendation: append-first journal with projected views.

Reason:

- Agent debugging requires causality.
- Memory should be rebuildable.
- Incidents need trace integrity.
- Evals need replay.

Do not make every byte append-only. Make authority, action, policy, receipt, and memory writes journaled.

### 8.4 Typed Actions vs Raw Tool Calls

Recommendation: typed actions.

Reason:

- Raw tool calls hide side effects.
- Policy cannot inspect arbitrary natural-language intent reliably.
- Human review needs diffs and structured risk.
- Simulation needs manifests.

Every tool invocation should map to a typed action class when possible.

### 8.5 Semantic Browser Service vs Pixel Automation Only

Recommendation: browser service with multiple observation modes.

Use:

- DOM where safe.
- Accessibility tree where useful.
- Screenshots for visual verification.
- Network/request metadata for audit.
- User approval for form submission and credentialed actions.

Pixel automation alone is too brittle and too hard to secure.

### 8.6 MCP/A2A Gateway vs Direct Trust In Tool Servers

Recommendation: gateway.

Reason:

- MCP standardizes context and tools, but protocol-level security alone is not enough.
- Remote tools can lie, change behavior, or request broad access.
- The OS needs policy enforcement, token binding, logging, and sandboxing at the boundary.

beaterOS should support MCP and A2A through a controlled gateway that converts external tool descriptions into local tool registry entries.

### 8.7 Stablecoin/Payment Primitive vs Crypto-First OS

Recommendation: payment primitive, not crypto-first.

Reason:

- Agent payments are important.
- Stablecoins and x402-style payment flows may matter for autonomous commerce.
- But tying the OS to one chain or payment rail creates avoidable risk.

The OS primitive should be a `PaymentMandate`:

- Who may spend.
- Asset or currency.
- Maximum amount.
- Counterparty constraints.
- Purpose.
- Time window.
- Approval rule.
- Idempotency.
- Receipt requirement.

Stablecoins, cards, bank APIs, invoices, and x402 can be adapters.

### 8.8 Post-Quantum Crypto Now vs Crypto Agility

Recommendation: crypto agility now, selective PQC adoption as libraries and ecosystems mature.

Reason:

- NIST finalized ML-KEM, ML-DSA, and SLH-DSA in 2024.
- Long-lived identities and signed audit logs should be designed for algorithm rotation.
- But inventing custom crypto or forcing PQC everywhere too early can reduce practical security.

Use standard libraries, standard protocols, and algorithm agility. Do not invent cryptography.

### 8.9 TEEs Everywhere vs Narrow Confidential Execution

Recommendation: narrow use.

Reason:

- TEEs can protect some secrets and workloads.
- They do not solve prompt injection, tool misuse, policy design, or user intent.
- They add operational complexity.

Use TEEs for:

- Secret handling.
- Remote attestation of critical services.
- High-value policy engines.
- Private model inference where feasible.

Do not make TEEs the core safety story.

### 8.10 Multi-Agent Society vs Small Number Of Principals

Recommendation: small number of principals first.

Reason:

- Multi-agent systems become hard to debug quickly.
- Most useful workflows need one main agent, tool services, occasional verifier, and human review.

Start with:

- User.
- Primary agent.
- Verifier agent.
- Tool service.
- Policy service.
- Human reviewer.

Add richer multi-agent delegation only after traces and capabilities are stable.

### 8.11 Memory As Database vs Memory As Projection

Recommendation: projection.

Reason:

- Memory needs provenance and rebuildability.
- A vector database is useful, but should not be the source of truth.
- Summaries and embeddings are lossy.

Source of truth: journaled events.

Derived views:

- Embeddings.
- Summaries.
- Preference profiles.
- Entity graphs.
- Project state.
- Recent working context.

### 8.12 Prompt Policy vs System Policy

Recommendation: system policy.

Reason:

- Prompt instructions are advisory.
- Models can be confused or manipulated.
- Policy must execute outside the model.

The model may propose. The policy service decides.

### 8.13 Local-First vs Cloud-First

Recommendation: local-first control plane, cloud-compatible execution.

Reason:

- Agent OS trust starts with user control.
- Local workspaces, secrets, and browser sessions need strict boundaries.
- Cloud execution is essential for scale, but should be an execution lane, not the root of authority.

### 8.14 Everything Is A File vs Everything Is An Object

Recommendation: object model with file projections.

Plan 9 showed the power of file-like interfaces. But agent-specific authority and provenance need richer metadata.

beaterOS should expose stable object IDs and allow filesystem views where convenient.

### 8.15 Full Formal Verification vs Practical Assurance First

Recommendation: practical assurance first, formal methods for the small TCB later.

Reason:

- Formal verification is powerful but expensive.
- The first TCB should be small enough to verify later.

Potential formal targets:

- Capability attenuation rules.
- Policy evaluation semantics.
- Journal hash chain integrity.
- Revocation invariants.
- Sandbox grant construction.

### 8.16 Shared Resources: Leases vs Unmanaged Conflict

Recommendation: leases as grant constraints.

Reason:

- Two sessions or a parent and subagent can hold overlapping write grants on the same workspace path, repo, cloud resource, or memory scope, and nothing in the contracts serializes their exercise.
- Lost updates and torn workspace state cannot be reconstructed by replay, because the journal orders events per session, not across sessions touching the same resource.
- Classical OSs answer this with locks, leases, and transactions; an agent OS cannot skip the question.

Implementation principle:

- A write grant may carry an exclusivity mode (exclusive or shared), making the capability service the lock manager: issuing an exclusive-write grant against a live overlapping grant queues, attenuates to read-only, or requires explicit policy override.
- Leases reuse the existing grant lifecycle: expiry is the lease timeout, revocation is the lease break. No separate lock service.
- Receipt chains are per actor, with the session journal periodically anchoring all actor chain heads into one Merkle node. Verification is per-chain plus anchors. This removes the chain-head serialization point that a single linear chain would impose on concurrent subagents. (Issue #47.)

### 8.17 Durable Execution: Adopt vs Rebuild

Recommendation: adopt a durable-execution engine for session state; keep the audit journal bespoke.

Reason:

- Pausable, resumable, replayable, cancelable-with-compensation sessions with journal-before-side-effect and idempotency keys are, word for word, the durable-execution feature set that Temporal, Restate, and DBOS ship today, with documented agent integrations.
- These engines give exactly-once state transitions plus at-least-once side effects made safe by idempotency — precisely the compromise the ActionManifest idempotency key already accepts.
- The journal serves two masters that must not be conflated: execution durability (resume, retry, compensate — commodity, buy it) and authority/audit integrity (hash-linked receipts, policy decisions, tamper evidence — the actual invention, keep it small and bespoke). Conflating them inflates the TCB with commodity distributed-systems problems: replay determinism, workflow versioning, replay-breaking code changes.

If the decision is build-anyway — defensible for a local-first Rust kernel with a small dependency budget — the plan must say so explicitly and scope what is deliberately not supported, e.g. sessions resume from receipts, not from replayed model calls. (Issue #52.)

## 9. Proposed Architecture

beaterOS should be organized as layered services.

### 9.1 Layer 0: Existing Substrate

This layer is not owned by beaterOS at first.

Substrates:

- Linux.
- macOS.
- Cloud VMs.
- Containers.
- Web browsers.
- Existing filesystems.
- Existing identity providers.
- Existing model APIs.
- Existing local models.

beaterOS should use the substrate, not pretend it does not exist.

### 9.2 Layer 1: Agent Kernel

Core daemon responsibilities:

- Session creation.
- Capability issuance.
- Policy evaluation.
- Action admission.
- Journal writes.
- Receipt verification.
- Revocation.
- Audit query.
- Eval gate enforcement.

This is the smallest trusted core.

### 9.3 Layer 2: Service Fabric

System services:

- Tool gateway.
- Browser service.
- Shell/code sandbox.
- Memory service.
- Model router.
- Observability service.
- Simulation/eval service.
- Human review service.
- Payment service.
- Package/tool registry.

Each service should be least-privileged and mediated through capabilities.

### 9.4 Layer 3: Workspaces

A workspace is a scoped environment for work.

Workspace includes:

- Files.
- Repos.
- Browser contexts.
- Secrets references.
- Project memory.
- Tool set.
- Eval scenarios.
- Policies.
- Team members.
- Audit logs.

Workspaces should have clear import/export and archival semantics.

### 9.5 Layer 4: Interfaces

Interfaces:

- CLI.
- TUI.
- Desktop app.
- Browser extension.
- API.
- MCP server.
- A2A endpoint.
- Web dashboard.

The first interface can be a CLI and web dashboard. The architecture should not depend on a chat UI.

## 10. Core Components

### 10.1 `beater-osd`: Agent Kernel Daemon

Responsibilities:

- Owns root journal.
- Owns policy versions.
- Issues capability grants.
- Registers agents.
- Validates action manifests.
- Accepts receipts.
- Publishes trace events.
- Exposes local API.

Design rule: keep it small. It should not run arbitrary tools directly.

### 10.2 Capability Service

Responsibilities:

- Create grants.
- Attenuate grants.
- Revoke grants.
- Check grants.
- Explain grants.
- Bind grants to sessions and principals.

Capability checks must be fast, deterministic, and independent of the model.

### 10.3 Policy Engine

Responsibilities:

- Evaluate action manifests.
- Classify risk.
- Require approvals.
- Require simulation.
- Deny overbroad actions.
- Enforce budget and data policy.
- Record explanations.

Policies should be versioned, testable, and diffable.

Candidate policy languages:

- Rego/OPA-style policies.
- Cedar-style authorization policies.
- A small custom DSL only if existing systems cannot express agent-specific constraints.

Recommendation: start with a proven policy engine and wrap it in agent-specific schemas.

Specifically: Cedar for admission policy. The selection criteria are this document's own requirements, and they point one way. Admission is exactly PARC-shaped — may this principal perform this action on this resource in this context — which is the shape Cedar is purpose-built for. Cedar is default-deny with forbid-overrides-permit as language semantics; evaluation is total, loop-free, and free of external calls, with microsecond-class decisions; the authorizer and validator are formally modeled in Lean 4 with mechanized proofs and differential testing against the production Rust implementation; and SMT-based analysis makes "does policy set A permit anything B does not" a decision procedure — policy diffing becomes mechanical, which is the section 20.5 sprawl mitigation. Rego remains available for non-TCB infrastructure policy; full Rego cannot be analyzed this way (Datalog program equivalence is undecidable). A custom DSL is rejected: it is inventing cryptography, applied to authorization. This choice also collapses the section 8.15 formal target for policy evaluation semantics into inherited upstream proofs. (Issue #51.)

### 10.4 Journal Service

Responsibilities:

- Append session events.
- Hash-link receipts.
- Store digests of large artifacts.
- Support redaction without destroying integrity.
- Support replay exports.
- Support transparency log integration.

Journal event classes:

- Session created.
- Capability granted.
- Action proposed.
- Policy decision.
- Human review.
- Tool started.
- Tool finished.
- Memory write.
- Model call.
- External side effect.
- Eval result.
- Incident annotation.

### 10.5 Tool Gateway

Responsibilities:

- Normalize tools from MCP, A2A, OpenAPI, CLIs, local functions, browser actions, and cloud SDKs.
- Validate schemas.
- Attach risk metadata.
- Require capability grants.
- Log inputs and outputs with redaction.
- Prevent token passthrough.
- Enforce network and data policy.

The gateway is where most agent frameworks are too loose today.

### 10.6 Sandbox Service

Execution lanes:

- Pure function lane: deterministic local functions, no network, no filesystem except mounted inputs.
- WASI/WebAssembly lane: portable sandboxed tools.
- Container lane: code execution with seccomp/AppArmor/cgroups and network controls.
- Browser lane: isolated browser profiles and origin policies.
- VM lane: high-risk or kernel-adjacent tasks.
- Remote tool lane: MCP/A2A/OpenAPI tools mediated by gateway.

Every lane must emit receipts.

### 10.7 Browser Service

Responsibilities:

- Manage isolated browser contexts.
- Capture observations.
- Classify origins.
- Protect credentials.
- Prevent silent form submission where policy forbids it.
- Produce receipts for navigation, clicks, forms, downloads, uploads, and purchases.
- Support simulation fixtures.

For agentic browsing, the browser is a security boundary, not just a UI surface.

### 10.8 Memory Service

Responsibilities:

- Store memory records.
- Build projections.
- Serve context with provenance.
- Enforce retention.
- Support redaction.
- Score confidence.
- Separate data classes.
- Prevent untrusted content from becoming privileged instruction.

Memory should be queryable by both semantic search and structured provenance filters.

### 10.9 Model Router

Responsibilities:

- Choose model based on task, risk, data class, latency, cost, and policy.
- Support local and cloud providers.
- Track model versions.
- Track provider retention constraints.
- Redact or transform inputs where required.
- Route verifier and planner calls separately.
- Record model call metadata.

Do not put policy inside the model router. The router obeys policy.

### 10.10 Human Review Service

Responsibilities:

- Create review requests.
- Present concise diffs and side-effect previews.
- Record approvals and denials.
- Enforce reviewer permissions.
- Escalate stale reviews.
- Support multi-party approval for high-value actions.

Human review should be integrated into the journal.

### 10.11 Eval And Simulation Service

Responsibilities:

- Run scenario manifests.
- Control deterministic fixtures.
- Compare traces.
- Score outcomes.
- Detect regressions.
- Gate releases.
- Produce eval reports.

The eval service should be able to replay production incidents as tests.

### 10.12 Observability Service

Responsibilities:

- Emit OpenTelemetry-compatible traces.
- Link spans to sessions, actions, tools, policies, and receipts.
- Track cost, latency, failure, and risk.
- Provide incident timeline views.
- Support audit export.

For Beater alignment, this is where existing trace and eval patterns can become core OS behavior.

### 10.13 Payment Service

Responsibilities:

- Manage payment mandates.
- Enforce spend limits.
- Bind payments to sessions.
- Require approval for high-risk transactions.
- Support idempotency and receipts.
- Integrate multiple rails.

Stablecoins and x402 should be adapters, not foundations.

### 10.14 Package And Tool Registry

Responsibilities:

- Register tools and agents.
- Store signed manifests.
- Track versions.
- Track test status.
- Track risk class.
- Track required capabilities.
- Support quarantine and revocation.

An agent OS without a trustworthy tool registry will inherit every supply-chain problem in the ecosystem.

## 11. Beater Ecosystem Fit

The current Beater direction appears aligned with an agent OS if the OS is treated as a control plane.

Potential local building blocks:

- `beater-agents`: trace, dataset, eval, gate, and monitor loop.
- `beatbox`: sandbox execution primitive.
- `beater-memory`: ledger-derived memory projection.
- `tempo`: agent-native browser surface.
- Generated SDK/MCP/CLI contracts: integration boundary for tools and external agents.

Recommended mapping:

- beaterOS owns contracts and authority.
- `beatbox` executes sandboxed actions.
- `tempo` executes browser actions.
- `beater-memory` serves memory projections from journaled events.
- `beater-agents` runs evals, monitors, and gates.
- MCP/CLI/SDK layers expose controlled interfaces.

This avoids building a separate universe. beaterOS becomes the thing that binds existing Beater pieces into an agent-first operating layer.

## 12. Core Data Contracts

These are conceptual contracts, not implementation code.

### 12.1 AgentSession

Purpose: represent one goal-directed run.

Required fields:

- `session_id`
- `created_at`
- `created_by`
- `agent_id`
- `workspace_id`
- `goal`
- `constraints`
- `policy_profile`
- `initial_capability_ids`
- `budget`
- `model_policy`
- `memory_scope`
- `journal_root`
- `status`

Important invariants:

- A session cannot execute actions without at least one capability grant.
- A session status transition is journaled.
- A session can be paused and resumed without losing causality.
- A session can be exported for audit with redactions.
- Execution-durability state (resume points, retries, compensation progress) is distinct from audit-journal events; the audit journal is authoritative and the execution engine emits into it (section 8.17).

### 12.2 CapabilityGrant

Purpose: represent explicit authority.

Required fields:

- `grant_id`
- `issuer`
- `holder`
- `session_id`
- `resource`
- `actions`
- `constraints`
- `expires_at`
- `delegation`
- `approval_requirements`
- `revocation_handle`
- `policy_version`
- `reason`

Important invariants:

- Grants cannot be broadened by the holder.
- Delegated grants must be equal or narrower.
- Expired grants fail closed.
- Revoked grants fail closed.
- Grants are never inferred from prompt text.

### 12.3 ActionManifest

Purpose: predeclare a proposed side effect or observation.

Required fields:

- `action_id`
- `session_id`
- `tool_id`
- `action_type`
- `target`
- `inputs_digest`
- `inputs_summary`
- `expected_outputs`
- `expected_side_effects`
- `required_grants`
- `risk_class`
- `data_classes`
- `idempotency_key`
- `compensation_plan`

Important invariants:

- Risk class can be raised by policy, never lowered by the agent.
- Unknown side effects require denial or review.
- Actions that affect external state must have receipts.
- No policy rule may condition on an agent-asserted field. Policy predicates consume only kernel-derived fields (section 7.4). This is checkable mechanically against the policy pack.
- Divergence between manifested and observed behavior freezes the grant and raises an incident event.

### 12.4 PolicyDecision

Purpose: deterministic admission result.

Required fields:

- `decision_id`
- `action_id`
- `policy_version`
- `result`
- `matched_rules`
- `explanation`
- `required_review`
- `required_simulation`
- `created_at`

Important invariants:

- Denied actions cannot be executed.
- Review-required actions cannot execute before approval.
- Policy decisions are journaled before execution.

### 12.5 CapabilityReceipt

Purpose: record what happened.

Required fields:

- `receipt_id`
- `action_id`
- `tool_id`
- `started_at`
- `finished_at`
- `status`
- `input_digest`
- `output_digest`
- `side_effect_summary`
- `external_ids`
- `artifact_refs`
- `previous_receipt_hash`
- `receipt_hash`

Important invariants:

- Receipts are append-only.
- Large artifacts can be content-addressed.
- Sensitive fields can be redacted through references without breaking the receipt chain.
- Receipt chains are per actor; concurrent subagents never contend on a single chain head. Session-level integrity comes from Merkle anchors over all actor chain heads (section 8.16).
- Durability is tiered by risk class (section 13.11): irreversible or external side effects require the intent record and policy decision synchronously durable before execution; reversible workspace mutations may group-commit; pure observations may journal asynchronously with per-actor ordering preserved.

### 12.6 MemoryRecord

Purpose: preserve knowledge with provenance.

Required fields:

- `memory_id`
- `source_event_id`
- `source_digest`
- `writer`
- `created_at`
- `kind`
- `content_ref`
- `summary`
- `confidence`
- `sensitivity`
- `expiry`
- `access_policy`

Important invariants:

- Memory has a source.
- Memory has an access policy.
- Memory can be invalidated.
- Derived memory can be rebuilt.

### 12.7 PaymentMandate

Purpose: limit economic authority.

Required fields:

- `mandate_id`
- `issuer`
- `holder`
- `session_id`
- `rail`
- `asset`
- `max_amount`
- `counterparty_policy`
- `purpose`
- `expires_at`
- `approval_threshold`
- `idempotency_key`
- `receipt_requirement`

Important invariants:

- No payment without a mandate.
- No silent mandate expansion.
- All payment attempts produce receipts.

### 12.8 ScenarioManifest

Purpose: make tasks testable.

Required fields:

- `scenario_id`
- `goal`
- `environment`
- `fixtures`
- `allowed_tools`
- `forbidden_actions`
- `seed_data`
- `oracle`
- `success_criteria`
- `risk_traps`
- `budget`
- `expected_trace_properties`

Important invariants:

- Scenarios are versioned.
- Scenario results are comparable across model and policy versions.
- Production incidents become new scenarios.

## 13. Security Model

Security is the most important differentiator. A weakly secured agent OS is worse than no agent OS because it concentrates authority behind persuasive natural language.

### 13.1 Root Security Principle

The model is never the root of trust.

The model may:

- Propose.
- Summarize.
- Classify.
- Draft.
- Explain.
- Verify as one signal.

The model must not:

- Grant itself authority.
- Bypass policy.
- Decide its own audit level.
- Hide side effects.
- Convert untrusted data into privileged instructions.
- Spend money without a mandate.
- Exfiltrate secrets because text asked it to.

### 13.2 No Ambient Authority

Ambient authority is the default failure mode.

beaterOS must prevent:

- Global filesystem access.
- Global shell access.
- Global browser cookies.
- Global network access.
- Global cloud credentials.
- Global MCP credentials.
- Global secrets in environment variables.
- Global payment credentials.

Every dangerous resource must require an explicit capability grant.

Authority boundaries are placed where the agent cannot route around them — the tool gateway, the sandbox, network egress — never inside code the agent controls. Cooperative APIs are conveniences layered on top of non-bypassable mediation. This is the section 8.12 argument applied one layer up: prompts are advisory to models, and SDK conventions are advisory to runtimes; both times the fix is enforcement at a boundary the untrusted party cannot decline. (Issue #54.)

### 13.3 Capability Design

Good grants are:

- Narrow.
- Time-limited.
- Resource-specific.
- Action-specific.
- Revocable.
- Delegation-aware.
- Auditable.
- Human-legible.

Bad grants look like:

- "Can access the computer."
- "Can use the browser."
- "Can call tools."
- "Can read files."
- "Can use production."
- "Can spend."

The UI and policy engine should make bad grants hard to create.

### 13.4 Taint And Provenance

Every information source should carry labels.

Useful labels:

- `trusted_user_instruction`
- `system_policy`
- `developer_instruction`
- `untrusted_web`
- `untrusted_email`
- `untrusted_document`
- `tool_output`
- `secret`
- `personal_data`
- `customer_data`
- `financial_data`
- `code`
- `binary`
- `payment_instruction`

Policy examples:

- Untrusted web content cannot create new tool permissions.
- Untrusted email content cannot authorize payments.
- Tool output cannot silently become system instruction.
- Secrets cannot be sent to external models unless a policy permits that data class.
- Customer data cannot enter public model routes.

### 13.5 Prompt Injection Defense

Prompt injection is not solved by better prompting.

Required defenses:

- Data/instruction separation.
- Tool allowlists.
- Capability checks outside the model.
- Output validation.
- Human review for risky side effects.
- Memory quarantine for untrusted sources.
- Browser origin policy.
- Content security labels.
- Refusal to execute instructions discovered in untrusted content unless explicitly promoted by a trusted user.

The defenses above are detective: they inspect what a persuaded model already decided to do, and Google's deployed experience defending Gemini shows that classifier-and-validation layers fail under adaptive attack. The primary defense is structural: fix the control flow before the agent reads untrusted content, so injected text can influence values but never which actions run. The 2025 literature converged here — CaMeL (a privileged model plans from trusted instructions only; a quarantined model parses untrusted data into typed values with no tool access; an interpreter enforces per-value capability policies, achieving 77% of AgentDojo tasks with provable security), Microsoft FIDES (deterministic information-flow labels in the planner), and the six design patterns of Beurer-Kellner et al.: action-selector, plan-then-execute, LLM map-reduce, dual LLM, code-then-execute, context-minimization.

beaterOS should expose these as session execution modes — open-loop (the full detective stack above), plan-then-execute (the tool-call plan is admitted by policy before the agent reads untrusted content; later untrusted input cannot add actions), and dual-LLM/code-then-execute (CaMeL-style quarantined parsing) — and let policy require stronger modes for riskier grants, e.g. sessions holding spend or communicate capabilities must run plan-then-execute or stronger. In plan-then-execute mode, action manifests are simply admitted as a batch up front. The known CaMeL costs — hand-written data-flow policies and confirmation fatigue — are exactly what policy packs (section 20.5) and the human review service (section 10.10) exist to absorb, so the OS is the natural home for the pattern. (Issue #48.)

### 13.6 MCP And Remote Tool Security

MCP is important, but beaterOS should not trust arbitrary MCP servers directly.

Required controls:

- OAuth token audience binding for remote MCP.
- No token passthrough.
- Explicit resource indicators.
- Tool schema pinning.
- Tool description distrust by default.
- Remote server identity verification.
- Tool allowlists per workspace.
- Policy checks per invocation.
- Input/output redaction.
- Rate limits.
- Egress restrictions.
- Revocation and quarantine.

Remote tools should be treated like network services with code execution implications.

### 13.7 Browser Security

Browser agents are dangerous because browsers combine identity, payments, communication, and untrusted content.

Required controls:

- Isolated browser profiles per session or workspace.
- Cookie boundary enforcement.
- Download quarantine.
- Upload approval for sensitive files.
- Form submission review where required.
- Payment and purchase review.
- Phishing/lookalike origin detection.
- DOM and accessibility snapshots for audit.
- Screenshots for visual confirmation.
- Credential-use receipts.

The browser should default to observation. Side effects need policy.

### 13.8 Shell And Code Execution Security

Shell access should be a special execution lane.

Required controls:

- Workspace-scoped filesystem mounts.
- No inherited global secrets.
- Network off by default.
- seccomp/AppArmor/cgroups on Linux where available.
- Container or VM isolation for untrusted code.
- CPU/memory/time limits.
- Filesystem diff receipts.
- Dependency download policy.
- Binary execution restrictions.
- Build artifact quarantine.

The agent should not receive a raw shell with the user's full environment.

### 13.9 Secrets

Secrets must not live in prompts or normal logs.

Required controls:

- Secret handles instead of secret values.
- Tool-mediated secret use.
- Redaction in traces.
- Scope by session and tool.
- Rotation support.
- Leak detection.
- No external model exposure unless explicitly allowed.
- Receipts that prove a secret was used without exposing the secret.

### 13.10 Supply Chain

Required controls:

- Signed tool manifests.
- Pinned dependencies.
- Reproducible build support where practical.
- SBOMs.
- SLSA-style provenance.
- Sigstore/Rekor-style transparency for artifacts.
- Vulnerability scanning.
- Tool quarantine.
- Version rollback.

Agents amplify supply-chain risk because they can autonomously select and compose tools.

### 13.11 Tamper-Evident Logs

beaterOS should use hash-linked journals and Merkle-style batching.

Use this for:

- Receipt chains.
- Eval reports.
- Tool registry changes.
- Policy versions.
- Release attestations.

Do not confuse tamper evidence with secrecy. Merkle trees help detect modification. They do not make bad policy safe.

Journal durability is tiered by risk class, because a synchronous fsync'd hash-chained write on every action — including the read-only observations that dominate real traces — makes the governed path visibly slower than the bare one, and a bypassed safety layer provides zero safety:

- Irreversible or external side effects (spend, communicate, deploy, submit, writes outside the workspace): intent record and policy decision synchronously durable before execution. Non-negotiable.
- Reversible workspace mutations: journal write ordered before the effect but group-committed; a crash may lose the tail, and the filesystem diff receipt reconstructs it.
- Pure observations: asynchronous batched journaling, per-actor ordering preserved, durability eventual.

Hash-link at receipt granularity and Merkle-batch at anchor points, not per journal event. The safety claim is preserved exactly where it matters: nothing irreversible happens un-journaled, and the common case stays off the fsync path. Mediation overhead versus an ungoverned agent on the same scenarios is a first-class regression gate (section 14.6). (Issue #53.)

### 13.12 Cryptography

Principles:

- Use standard algorithms.
- Use maintained libraries.
- Rotate keys.
- Separate signing from encryption.
- Support algorithm agility.
- Prefer boring, audited crypto.
- Do not invent primitives.

Near-term:

- Ed25519/ECDSA or platform-standard signatures where appropriate.
- TLS with current best practices.
- Content-addressed artifacts.
- Sigstore-style artifact signing where useful.

Post-quantum path:

- Design identities and logs for algorithm agility.
- Track NIST ML-KEM, ML-DSA, and SLH-DSA support.
- Use hybrid modes for long-lived confidentiality once supported by mainstream stacks.
- Prioritize PQC for long-lived agent identity and audit artifacts before ephemeral low-value traffic.

### 13.13 Confidential Computing

Use TEEs selectively.

Good uses:

- Policy service attestation.
- Secret broker attestation.
- Sensitive inference.
- High-value transaction signing.

Bad uses:

- Treating a TEE as a fix for prompt injection.
- Hiding all execution in opaque enclaves.
- Making debugging impossible.

### 13.14 Human Approval Security

Human approval can fail if the approval UI is vague.

Good approval prompts show:

- Exact resource.
- Exact action.
- Diff or preview.
- Risk class.
- Agent explanation.
- Policy reason.
- Reversibility.
- Cost.
- External recipients or counterparties.

Bad approval prompts say:

- "Allow agent to continue?"
- "Approve tool use?"
- "Permit browser action?"

### 13.15 Incident Response

beaterOS needs an incident mode.

Capabilities:

- Freeze session.
- Revoke grants.
- Snapshot workspace.
- Quarantine tools.
- Preserve journal.
- Mark memory contaminated.
- Export trace.
- Generate incident timeline.
- Create regression scenarios.
- Rotate secrets.

Every serious incident should improve the eval suite.

## 14. Simulation And Evals

The eval layer is how beaterOS becomes reliable instead of impressive in demos.

### 14.1 Eval Philosophy

Agents must be evaluated on complete workflows, not only model answers.

Measure:

- Task success.
- Side-effect correctness.
- Permission minimality.
- Data handling.
- Cost.
- Latency.
- Human interventions.
- Recovery.
- Auditability.
- Robustness to adversarial content.

### 14.2 Scenario Types

Required scenario classes:

- Filesystem tasks.
- Code editing tasks.
- Browser research tasks.
- Browser form tasks.
- Email/chat drafting tasks.
- Cloud administration tasks.
- Payment/purchase tasks.
- Data analysis tasks.
- Multi-step business workflow tasks.
- Prompt injection tasks.
- Malicious tool tasks.
- Compromised document tasks.
- Memory poisoning tasks.
- Model downgrade tasks.
- Network failure tasks.
- Human review timeout tasks.

Bespoke scenarios are necessary but not sufficient. The security suite must also run a public external benchmark — AgentDojo (97 tasks, 629 injection cases; the field's standard for agent injection resistance, used by CaMeL, FIDES, and Progent) — so that "agent under beaterOS vs the same agent bare, utility and attack-success-rate deltas" is a falsifiable public claim rather than a self-graded one. (Issue #48.)

### 14.3 Oracle Ladder

Not every task can be scored the same way.

Oracle types:

- Exact match.
- Unit tests.
- Static checks.
- Diff checks.
- State checks.
- API assertions.
- Browser DOM assertions.
- Screenshot comparisons.
- Human rubric.
- LLM-as-judge with calibration.
- Multi-oracle consensus.

Use the weakest oracle that is reliable enough. Avoid using an LLM judge as the only oracle for high-stakes workflows.

### 14.4 Trace-Based Metrics

Outcome alone is insufficient.

Trace metrics:

- Number of actions.
- Number of denied actions.
- Number of approval requests.
- Number of overbroad grant attempts.
- Secret exposure attempts.
- Untrusted instruction violations.
- Tool retries.
- Replanning count.
- Model calls.
- Cost.
- Time to first useful action.
- Time to completion.
- Reproducibility score.
- Receipt completeness.

### 14.5 Security Evals

Security evals must be adversarial.

Examples:

- Web page attempts prompt injection.
- PDF contains hidden instructions.
- MCP server exposes lookalike tool.
- Tool output asks for a secret.
- Email asks agent to wire funds.
- Browser origin imitates trusted site.
- Model suggests broad permission.
- Subagent requests parent authority.
- Memory contains poisoned fact.
- Payment address changes mid-flow.

Success means the OS policy layer blocks or escalates, even if the model is persuaded.

### 14.6 Regression Gates

Every release should run:

- Fast smoke scenarios.
- Core workflow scenarios.
- Security scenarios.
- Cost regression checks.
- Latency regression checks.
- Held-out scenarios.
- Incident replay scenarios.

Model upgrades must run paired evals:

- Same scenario.
- Same tools.
- Same policy.
- New model route.
- Compare success, cost, risk, and trace properties.

Release gates also include an overhead regression check: the same scenarios run with and without beaterOS mediation, and the wall-clock and cost delta must stay within budget (section 13.11).

### 14.7 Counterfactual Replay

A strong OS can ask:

- What if this grant had been narrower?
- What if this model had been cheaper?
- What if this tool had failed?
- What if this web page contained injection?
- What if human approval timed out?
- What if this memory was unavailable?

Counterfactual replay is a research differentiator. It turns production traces into design insight.

### 14.8 Simulation Environments

Use the simplest environment that captures the risk.

Environment types:

- Local temp workspace.
- Containerized service mesh.
- Browser fixture.
- Mock SaaS.
- Fake payment rail.
- VM snapshot.
- Cyber range.
- Mobile emulator.
- Hardware/robotics simulator later.

Do not run high-risk tasks against production first.

### 14.9 Statistical Method

Agents are probabilistic, so eval gates decided on single-run point estimates pass flaky agents on lucky draws and flag sampling noise as regressions. Each scenario run is a Bernoulli trial and the gate must treat it that way:

- Reliability gates use pass^k — the probability that all k trials succeed — not pass@1. A 90%-per-run agent is roughly 43% at pass^8; tau-bench measured then-SOTA agents below 25% pass^8 on retail tasks while single-run rates looked respectable. Choose k per risk class: k=1 for smoke, k=4 for core workflows, k=8 for scenarios guarding irreversible actions. Report both metrics.
- Release comparisons are paired: same scenarios, per-scenario deltas, clustered standard errors (scenario packs share fixtures and are correlated), and a pre-declared minimum detectable effect. A gate fails on a statistically supported regression, not on any negative point delta.
- The eval runner supports sequential stopping: stop once the confidence interval clears the gate threshold. This is what makes multi-sampling affordable — agent runs are expensive, and sequential designs cut trial counts substantially in A/B practice.
- Trace metrics (section 14.4) report distributions — p50, p95, p99 — not means.

(Issue #50.)

## 15. Next-Generation Model Support

beaterOS must support models that are more capable than today's systems without giving them unchecked authority.

### 15.1 Long Context

Long context changes the OS problem:

- More data can fit in a prompt.
- More secrets can accidentally enter a prompt.
- More stale context can influence behavior.
- More untrusted instructions can be mixed with trusted instructions.

beaterOS must apply context construction policy:

- What data is allowed.
- Which source labels are included.
- Which summaries are used.
- Which secrets are replaced with handles.
- Which provenance is attached.
- Which memory is excluded.

### 15.2 Stateful Models

If providers expose stateful sessions, the OS must track:

- Provider session ID.
- Data retention.
- State reset.
- Memory boundaries.
- Audit export.
- Revocation or deletion semantics.

Stateful model sessions must not become hidden memory outside beaterOS governance.

### 15.3 Multimodal Models

Multimodal inputs create new risks:

- Screenshots can contain secrets.
- Images can contain hidden text.
- Audio can contain private speech.
- Video can contain bystanders or credentials.

Policies must classify multimodal data before routing.

### 15.4 Computer-Use Models

Computer-use models need:

- Browser and desktop sandboxes.
- Action previews.
- Coordinate-to-semantic mapping.
- Visual receipts.
- Form submission gates.
- Credential-use controls.

The OS should not let a computer-use model operate a user's real desktop with broad authority.

### 15.5 Local Models

Local models are important for:

- Privacy.
- Low latency.
- Cost control.
- Offline operation.
- Sensitive classification.
- Simple verifiers.

The model router should treat local models as first-class, even if frontier models remain necessary for hard tasks.

### 15.6 Verifier Models

Verifier models should be separate from planner/executor models when possible.

Use cases:

- Detect overbroad permissions.
- Check summaries.
- Check policy explanations.
- Classify data sensitivity.
- Review diffs.
- Detect prompt injection attempts.

Verifier output is still advisory. Policy remains deterministic.

### 15.7 Model Cost And Latency

The OS should optimize:

- Cheap model for simple classification.
- Strong model for planning.
- Specialized model for code.
- Local model for sensitive data.
- Fast model for interaction.
- Verifier model for high-risk action.

Cost is an OS resource.

### 15.8 Private Reasoning Traces

Some models do not expose full reasoning traces. beaterOS should not require them.

Instead, require:

- Action manifests.
- Tool inputs.
- Tool outputs.
- Summaries.
- Receipts.
- Policy decisions.
- Reproducible context assembly.

The OS should audit behavior, not depend on hidden chain-of-thought.

## 16. Crypto, Stablecoins, And Quantum

These areas matter, but they must be subordinated to the OS design.

### 16.1 Stablecoins And Agent Payments

Agent payments are likely to matter because agents will:

- Buy APIs.
- Pay for data.
- Pay for compute.
- Execute microtransactions.
- Use usage-based services.
- Purchase goods.
- Settle between services.

But the core OS abstraction should not be "stablecoin". It should be "bounded payment authority".

Required payment controls:

- Explicit mandate.
- Counterparty validation.
- Amount ceiling.
- Recurrence limit.
- Purpose binding.
- Human approval thresholds.
- Fraud checks.
- Idempotency.
- Receipts.
- Reconciliation.
- Revocation.

x402-style flows are interesting because they make payments HTTP-native, but they must pass through payment mandates and receipts.

### 16.2 Blockchain Uses That Might Matter

Potentially useful:

- Payment settlement.
- Public transparency logs.
- Decentralized identity anchors.
- Verifiable timestamping.
- Tool/package attestations.

Likely overkill early:

- Putting all logs on-chain.
- Token incentives for every action.
- Decentralized governance before product-market fit.
- Custom chain.
- NFT-style capability objects.

Use blockchains only where they solve a real trust boundary.

### 16.3 Quantum Threats

Quantum computing matters mainly for cryptography planning.

beaterOS should:

- Avoid long-lived RSA/ECDH assumptions for future-facing identity design.
- Support algorithm agility.
- Track NIST PQC standards.
- Prepare hybrid key exchange for long-lived channels.
- Prepare PQC signatures for long-lived artifacts.

beaterOS should not:

- Claim quantum security through novelty.
- Use experimental crypto in critical paths.
- Build QRNG dependency into core design.

### 16.4 Mathematical Modeling

Useful mathematical models:

- Capability lattices for authority attenuation.
- Information-flow labels for taint.
- Merkle DAGs for journal integrity.
- State machines for session lifecycle.
- Temporal logic for policy invariants.
- Queueing theory for scheduler and review bottlenecks.
- Bayesian confidence for memory reliability.
- Game theory for adversarial tools and prompt injection.
- Cost models for model routing.

The first formalization target should be authority and state transitions, not general agent intelligence.

## 17. Product Shape

The product should feel like an OS for serious agent work, not a chatbot.

### 17.1 First User

Best first user:

- Technical builder.
- Uses agents for code, research, browser work, and automation.
- Cares about traceability.
- Will tolerate CLI/dashboard experience.
- Has real workflows but can operate locally.

Avoid starting with:

- General consumer desktop replacement.
- Enterprise compliance-only product.
- Fully autonomous finance agent.
- Robotics.
- Medical/legal autonomous actions.

### 17.2 First Interface

Start with:

- CLI for sessions and grants.
- Local dashboard for traces, policies, reviews, and evals.
- Workspace configuration.
- Tool registry view.
- Eval runner.

Do not start with a full desktop OS shell.

The first deliverable is a drop-in mediation layer, not a native runtime: an MCP gateway proxy plus sandbox wrapper usable with at least one popular existing agent, unmodified. MCP is a universal interposition seam — a proxying gateway sees every tool call of any MCP-speaking agent, synthesizes the action manifest from the observed call (which is also what makes manifest fields trustworthy, section 7.4), runs admission, and emits receipts with zero agent-side changes. Enforcement that depends on the governed runtime's cooperation is not enforcement, and "point your existing agent at this proxy" is an afternoon where "rewrite against beaterOS APIs" is a migration. Native beaterOS-first agents remain the deep integration where cooperative features — plan-then-execute modes, richer manifests, memory provenance — light up. (Issue #54.)

### 17.3 First Killer Workflow

The strongest first workflow is probably software work:

- Read repo.
- Plan change.
- Edit files.
- Run tests.
- Use browser/docs.
- Ask for permission when needed.
- Produce patch and trace.
- Run eval gates.

Why:

- Filesystem side effects are easy to inspect.
- Tests exist.
- Developers understand diffs.
- Sandboxing is practical.
- Beater already appears aligned with agent evals and traces.

Second workflow:

- Browser research with citations and controlled downloads.

Third workflow:

- Internal operations with mock SaaS before real SaaS.

### 17.4 UX Principles

UX should prioritize:

- What is the agent trying to do?
- What is it allowed to do?
- What did it already do?
- What does it need from me?
- What changed?
- Can I replay or undo?
- Why did policy allow or deny this?

Avoid:

- Chat-only control.
- Vague permission prompts.
- Hidden background activity.
- Unbounded autonomy toggles.
- Logs that require model expertise to read.

## 18. Research Distillation

This plan is informed by OS history, agent papers, benchmarks, frontier company direction, security work, and OS development community guidance.

### 18.1 OS Design Lessons

Microkernels:

- Keep trusted core small.
- Move services out of kernel.
- Enforce message-passing boundaries.
- Relevant to beaterOS because the agent kernel should be minimal and service-oriented.

seL4:

- Demonstrates that a small kernel and capability model can be formally verified.
- Relevant as a long-term high-assurance path.

Exokernel:

- Separates protection from management.
- Lets applications define abstractions above low-level secure primitives.
- Relevant because agent workflows may need flexible policy and runtime behavior without weakening protection.

Plan 9:

- Shows the power of coherent namespaces.
- Relevant because beaterOS needs a uniform namespace for tools, memory, browser sessions, resources, and traces.

Singularity/Midori-style research:

- Explored managed-code isolation and software-isolated processes.
- Relevant because agent tools need strong isolation and verifiable contracts.

WASI/WebAssembly:

- Useful portable sandbox lane.
- Relevant for deterministic tools and plugin execution.

CHERI:

- Hardware capabilities for memory safety and compartmentalization.
- Relevant long-term for high-assurance local agents and tool isolation.

Linux security primitives:

- seccomp, cgroups, namespaces, LSMs, and eBPF are practical substrate controls.
- Relevant immediately for sandbox lanes.

2026 systems optimization lessons:

- Linux `sched_ext` shows that scheduler policy can be safely extended with BPF programs and turned on/off dynamically. beaterOS should use this as a Linux add-on experiment for policy-aware agent scheduling before designing a native scheduler.
- `io_uring` and its zero-copy receive work show the value of shared submission/completion rings, batching, and copy avoidance while keeping the kernel network stack in the path. beaterOS trace, receipt, and sandbox IO should use the same shape: bounded rings, explicit completion records, and minimal copies.
- XDP/eBPF show that safe, dynamically loaded programs at kernel hook points can handle packet filtering, tracing, and enforcement before expensive allocations. beaterOS should treat eBPF as a Linux enforcement and observability substrate, not as portable policy truth.
- DPDK and SPDK show when kernel bypass is justified: polled-mode queues, direct descriptors, zero-copy, and lockless paths can win for high-throughput devices, but only with explicit CPU, isolation, and audit budgets.
- Firecracker shows the value of minimal device models and fast microVM startup for workload isolation. beaterOS should prefer minimal virtual devices for risky agent lanes instead of exposing broad host surfaces.
- CXL/far-memory tiering shows that modern OS memory management is no longer just RAM versus disk. beaterOS memory must distinguish hot context, active working sets, cold provenance, embeddings, traces, and archives.
- Rust-for-Linux confirms that memory-safe kernel-adjacent code is now a serious systems path. beaterOS should default to Rust for all new low-level code unless an ABI, driver, or measured hot path requires C.

### 18.2 Agent OS Research Lessons

AIOS:

- Frames LLM agents as needing OS-like scheduling, context management, memory, storage, access control, and tool management.
- Reinforces that resource management belongs below individual agent apps.

OS-Copilot:

- Shows that agents can interact with general computer environments, but also exposes brittleness of GUI/OS control.

AOS papers:

- Argue for agentic control planes integrated with or above traditional OSs.
- Reinforces the user-space-first path.

Qualixar OS and similar orchestration systems:

- Show market pressure toward multi-agent orchestration layers.
- Risk: orchestration without deep authority and audit becomes fragile.

### 18.3 Benchmarks And Simulations

OSWorld:

- Tests agents in real computer environments.
- Shows that OS-level tasks are difficult and require robust observation/action loops.

OSWorld 2.0:

- Indicates the benchmark space is moving toward more realistic and harder OS interaction.

WebArena:

- Tests web agents in realistic websites.
- Relevant for browser task simulation.

BrowserGym:

- Provides browser agent evaluation infrastructure.
- Relevant for beaterOS browser service evals.

SWE-bench:

- Useful for coding-agent workflows.
- Should be adapted into beaterOS session/eval traces.

Generative Agents:

- Shows simulated agents with memory and reflection.
- Relevant as a warning: memory makes agents more believable, but OS safety needs provenance and policy.

Voyager:

- Shows skill accumulation in open-ended environments.
- Relevant to tool and skill registries, but skill acquisition must be permissioned.

### 18.4 Frontier Company Direction

OpenAI:

- Computer-using agents and ChatGPT agent systems show the shift toward browser/computer operation, tool use, and safety cards.
- beaterOS should assume computer-use models become common.

Anthropic:

- Computer use and MCP point to model-tool integration becoming standardized.
- beaterOS should support MCP, but not rely on MCP alone for safety.

Google:

- A2A indicates a future where agents communicate across vendors and organizations.
- beaterOS needs identity, authorization, and trace boundaries between agents.

AWS:

- Bedrock AgentCore emphasizes runtime, memory, identity, gateway, browser, code interpreter, and observability.
- This validates the component map but beaterOS should keep local-first and open contracts.

Microsoft:

- "Agentic OS" direction and taskbar integration show that operating systems will expose agents as native users of the environment.
- beaterOS should focus on deeper contracts, not only UI integration.

Cloudflare:

- Edge agent runtimes show demand for durable, network-native agent execution.
- beaterOS should support edge/cloud lanes.

NVIDIA:

- Distributed inference frameworks for reasoning models suggest model serving and routing become OS-scale infrastructure.
- beaterOS should keep model routing abstract.

### 18.5 Security Research Lessons

Agent security papers increasingly converge on the idea that agents need OS-like enforcement:

- Untrusted inputs can influence agents.
- Tool calls are side effects.
- Prompting is not enforcement.
- Capabilities and taint tracking are natural defenses.
- Auditability and deterministic policy are essential.

The key security conclusion:

> The agent OS must enforce safety outside the model, using explicit authority, provenance, sandboxing, and receipts.

### 18.6 Community Signal

OSDev communities repeatedly emphasize:

- Hardware support is a trap for beginners.
- Drivers dominate effort.
- Debugging kernels is hard.
- Undefined behavior and concurrency bugs are brutal.
- A toy kernel is not a usable OS.
- A serious OS requires toolchains, filesystems, networking, scheduling, security, and years of maintenance.

Agent communities repeatedly emphasize:

- Tool permissions are unclear.
- Agents are hard to debug.
- Browser automation is brittle.
- Memory is unreliable.
- Evaluation is weak.
- Prompt injection remains unsolved.
- Productionizing agents is harder than demos imply.

Therefore the practical synthesis is:

- Do not start with a hardware kernel.
- Do not stop at an agent framework.
- Build the missing OS layer above existing substrates first.

Anonymous forums and fast-moving social threads can reveal pain points, but they should not drive security architecture. Primary sources, papers, official specs, and reproducible experiments should win.

## 19. Roadmap

### Phase 0: Planning Repo

Goal: create the design base.

Deliverables:

- `README.md`.
- `final.md`.
- Source matrix.
- Open questions list.
- Initial glossary.

Done when:

- The big design choices are explicit.
- Non-goals are explicit.
- First implementation contracts are clear.

### Phase 1: Contract-First Agent Kernel

Goal: define and validate the six core contracts.

Contracts:

- AgentSession.
- CapabilityGrant.
- ActionManifest.
- PolicyDecision.
- CapabilityReceipt.
- ScenarioManifest.

Deliverables:

- Schema definitions.
- Example traces.
- Policy examples.
- Threat model.
- Eval examples.

Done when:

- A session can be represented end-to-end.
- A tool action can be admitted or denied deterministically.
- A receipt chain can be verified.
- A simulated task can be scored.

### Phase 2: Minimal Local Runtime

Goal: run safe local workflows.

Capabilities:

- Create session.
- Grant scoped filesystem access.
- Run sandboxed command.
- Capture receipt.
- Record journal.
- Run simple eval.
- Show trace in dashboard or CLI.

Done when:

- A coding workflow can run in a temp workspace.
- The agent cannot access files outside its grant.
- Denied actions are blocked outside the model.
- The trace explains all side effects.

### Phase 3: Tool Gateway And MCP

Goal: integrate external tools safely.

Capabilities:

- Register tool manifests.
- Wrap MCP servers.
- Enforce OAuth/token rules for remote tools.
- Pin tool versions.
- Attach risk metadata.
- Capture tool receipts.

Done when:

- A remote tool can be used only through a grant.
- Tool description changes are detected.
- Token passthrough is blocked.
- Malicious tool evals fail closed.
- An unmodified third-party MCP agent, pointed at the gateway as a proxy, gains manifests, admission, and receipts with measured overhead and measured AgentDojo attack-success-rate deltas (section 17.2). The gateway is the adoption wedge, not an integration detail, and this phase co-leads with Phase 2 rather than strictly following it.

### Phase 4: Browser Service

Goal: safe browser automation.

Capabilities:

- Isolated browser profiles.
- DOM/accessibility/screenshot observations.
- Navigation receipts.
- Form submission policy.
- Download/upload policy.
- Prompt injection scenarios.

Done when:

- Browser research tasks pass.
- Credentialed actions require correct policy.
- Web prompt injection cannot grant authority.
- Browser actions are replayable enough to debug.

### Phase 5: Memory Service

Goal: accountable memory.

Capabilities:

- Memory records from journal events.
- Semantic retrieval with provenance.
- Redaction.
- Expiry.
- Confidence.
- Poisoning evals.

Done when:

- The system can answer "why do you know this?"
- Memory can be rebuilt from journal.
- Untrusted memory cannot become privileged instruction.

### Phase 6: Eval And Simulation Platform

Goal: make evals mandatory for change.

Capabilities:

- Scenario manifests.
- Deterministic fixtures.
- Browser scenarios.
- Tool failure scenarios.
- Security scenarios.
- Regression reports.

Done when:

- Every release runs eval gates.
- Model upgrades are compared.
- Production incidents become scenarios.

### Phase 7: Payment And Economic Controls

Goal: bounded economic agency.

Capabilities:

- Payment mandates.
- Spend limits.
- Receipts.
- Stablecoin/x402 adapter.
- Card/bank/invoice adapter later.
- Fraud and counterparty checks.

Done when:

- Agents cannot spend without mandate.
- Payments are traceable to sessions and approvals.
- Payment simulations exist before real rails.

### Phase 8: Hardened Distribution

Goal: package the runtime for real users.

Capabilities:

- Signed releases.
- Secure auto-update.
- Local dashboard.
- Policy templates.
- Workspace templates.
- Tool registry.
- Team mode.

Done when:

- A developer can install and run beaterOS locally.
- The default profile is safe.
- Audit export works.

### Phase 9: High-Assurance Research Track

Goal: shrink and harden the trusted base.

Research:

- Formal capability model.
- Verified policy invariants.
- seL4 deployment prototype.
- CHERI compartment prototype.
- TEE-attested policy service.
- PQC signing for long-lived logs.

Done when:

- The research track improves real security without blocking normal product development.

## 20. Critical Open Questions

### 20.1 What Is The Smallest Useful Capability Grammar?

Need to define actions broadly enough to cover real workflows but narrowly enough for policy.

Candidate action families:

- Read.
- Write.
- Execute.
- Navigate.
- Submit.
- Communicate.
- Spend.
- Deploy.
- Remember.
- Delegate.
- Ask human.

### 20.2 What Should Be In The Trusted Computing Base?

Candidate TCB:

- Capability service.
- Policy engine.
- Journal verifier.
- Secret broker.
- Sandbox launcher.

Everything else should be less trusted.

### 20.3 How Much Of The Journal Should Be Local?

Tradeoff:

- Local journal improves privacy and user control.
- Cloud journal improves team audit and durability.

Recommendation:

- Local-first journal with optional encrypted sync and organization transparency log.

### 20.4 How Do We Make Human Approval Not Annoying?

Need:

- Risk-based approval thresholds.
- Good previews.
- Batch approvals only where safe.
- Learned preferences that do not grant new authority silently.
- Escalation policies.

### 20.5 How Do We Prevent Policy Sprawl?

Need:

- Small default policy profiles.
- Tested policy packs.
- Clear rule explanations.
- Policy diffing.
- Scenario coverage per policy.

### 20.6 What Is The Right Memory Default?

Recommendation:

- Short-lived working memory by default.
- Explicit promotion to long-term memory.
- Provenance required.
- Expiry required for sensitive data.

### 20.7 How Should Agents Delegate?

Recommendation:

- Delegation requires capability attenuation.
- Subagents get explicit task scopes.
- Parent agent remains accountable.
- Subagent traces link to parent session.

### 20.8 What Is A Safe Default Browser?

Recommendation:

- Fresh isolated browser context per high-risk session.
- No credentialed browsing unless user explicitly grants it.
- Downloads quarantined.
- Uploads reviewed.
- Purchases gated.

### 20.9 What Counts As A Side Effect?

Side effects include:

- File writes.
- Network writes.
- API mutations.
- Emails and messages.
- Browser form submissions.
- Downloads.
- Uploads.
- Memory writes.
- Payment attempts.
- Cloud changes.
- Ticket/comment creation.
- Model calls involving sensitive data.

Observation can also be risky if it reads sensitive data.

### 20.10 How Do We Avoid Becoming Another Agent Framework?

By enforcing boundaries that frameworks usually treat as conventions:

- Capability checks outside model.
- Journal before side effects.
- Receipts after side effects.
- Policy versioning.
- Eval gates.
- Tool registry.
- Memory provenance.
- Human review service.

## 21. Non-Goals

Near-term non-goals:

- Bare-metal kernel as the first implementation target.
- Broad hardware driver stack before agent workloads justify it.
- Full desktop environment.
- Custom browser engine.
- Custom cryptographic primitives.
- Blockchain-first architecture.
- Fully autonomous financial agent.
- Unbounded multi-agent society.
- Replacing Linux or macOS before the hosted agent kernel proves value.
- Solving general AGI alignment.

These are intentionally excluded from the first implementation phase to keep the project focused on the missing OS layer for agents.

Long-term, a metal-touching beaterOS is in scope when it is driven by evidence:

- The hosted runtime exposes a real workload the host kernel cannot mediate safely or fast enough.
- A low-level component has a crisp authority, memory, IO, scheduling, or audit boundary.
- The implementation has a macOS-compatible development path, a Linux compatibility story, and a simulator or hardware target.
- The subsystem can be tested with traces, benchmarks, property tests, or formal models before it carries user authority.

## 22. Failure Modes

### 22.1 It Becomes A Chat App

Symptom:

- Most control happens in prompts.
- Permissions are vague.
- Traces are transcripts.

Mitigation:

- Build around sessions, capabilities, manifests, policies, and receipts.

### 22.2 It Becomes A Kernel Hobby Project

Symptom:

- Work shifts to bootloaders, drivers, schedulers, and filesystems before agent contracts exist.

Mitigation:

- Use existing OS substrates until the agent kernel proves value.

### 22.3 It Becomes Too Enterprise-Heavy

Symptom:

- Compliance dashboards exist before local workflows work.

Mitigation:

- Start with developer workflows and real traces.

### 22.4 It Trusts MCP Too Much

Symptom:

- Remote tool servers can request broad access.
- Tool descriptions are trusted.

Mitigation:

- Gateway, pinning, policy, token binding, and receipts.

### 22.5 It Treats Memory As Magic

Symptom:

- Vector DB fills with untraceable summaries.

Mitigation:

- Memory records with provenance and rebuildable projections.

### 22.6 It Overuses LLM Judges

Symptom:

- Evals pass because a model says they pass.

Mitigation:

- Prefer deterministic oracles, use LLM judges only as calibrated support.

### 22.7 It Invents Crypto

Symptom:

- Custom hash/signature/capability scheme.

Mitigation:

- Standard algorithms, standard libraries, external review.

### 22.8 It Lets Agents Spend Too Soon

Symptom:

- Payment demos before mandates and simulations.

Mitigation:

- Fake rails first, then limited real rails with strict receipts.

### 22.9 It Cannot Explain Denials

Symptom:

- Policy blocks actions but users cannot understand why.

Mitigation:

- PolicyDecision explanations and matched rules.

### 22.10 It Has No Trusted Minimal Core

Symptom:

- Every service is equally trusted.

Mitigation:

- Define TCB and shrink it continuously.

## 23. Success Metrics

### 23.1 Safety Metrics

- Percentage of actions with valid manifests.
- Percentage of side effects with receipts.
- Number of ambient authority violations.
- Prompt injection block rate.
- Secret exposure rate.
- Overbroad grant request rate.
- Denied action bypass rate.
- Mean time to revoke compromised tool.

### 23.2 Reliability Metrics

- Task success rate.
- pass^k on the scenario suite at the risk-class k values of section 14.9.
- Pass rate on scenario suite.
- Regression rate after model upgrades.
- Replay success rate.
- Recovery success rate.
- Tool failure recovery rate.
- Human approval timeout handling.

### 23.3 Observability Metrics

- Trace completeness.
- Receipt completeness.
- Policy explanation coverage.
- Memory provenance coverage.
- Audit export completeness.

### 23.4 Cost Metrics

- Cost per successful task.
- Model cost by action class.
- Tool cost by workflow.
- Human review cost.
- Wasted retry cost.
- Admission decision latency (p99; microsecond-class policy evaluation plus journal append should keep this in single-digit milliseconds locally).
- Mediation overhead per action and as a percentage of task wall-clock versus an ungoverned baseline; the target must be small enough that nobody is tempted to disable governance.

### 23.5 UX Metrics

- Time to start a session.
- Time to understand what the agent did.
- Approval prompt clarity.
- False approval request rate.
- User correction rate.
- Time to debug failure.

### 23.6 Ecosystem Metrics

- Number of registered tools with tests.
- Number of scenario packs.
- Number of policy packs.
- Number of supported model providers.
- Number of sandbox lanes.
- Number of safe MCP integrations.

## 24. Minimum Viable beaterOS

The minimum viable system is not a desktop. It is a local runtime with strict contracts.

MVP capabilities:

1. Create an agent session from a goal.
2. Issue scoped file capability grants.
3. Register a small set of tools.
4. Require action manifests before tool calls.
5. Evaluate policy outside the model.
6. Run tools in a sandbox.
7. Append journal events.
8. Produce side-effect receipts.
9. Run a small scenario suite.
10. Show a trace with actions, grants, decisions, and receipts.

MVP proof:

- Give an agent a repo task.
- It can read only granted paths.
- It proposes edits.
- It runs tests in a sandbox.
- It cannot push, deploy, email, browse credentialed sites, or spend money without explicit grants.
- The final output includes a trace and receipts.
- The same task can be replayed in a scenario.

The MVP proof also has a wrapped form: an agent the project does not control, run unmodified through the beaterOS gateway and sandbox, gains manifests, policy admission, receipts, and injection resistance it did not have — with before/after utility, attack-success-rate, and overhead numbers. That is the falsifiable adoption claim. (Issue #54.)

## 25. What To Build First Later

When implementation starts, build in this order:

1. Schema package for core contracts.
2. Local append-only journal.
3. Capability checker.
4. Policy admission for action manifests.
5. Sandbox runner for one safe lane.
6. Receipt recorder.
7. CLI to create sessions/grants/run actions.
8. Scenario runner for one workflow.
9. Trace viewer.
10. Memory projection from journal.
11. MCP gateway.
12. Browser service.

Do not start with the UI. The UI should reveal the contracts, not replace them.

## 26. What Not To Compromise

Never compromise on:

- No ambient authority.
- Journal before side effects.
- Receipts after side effects.
- Policy outside the model.
- Memory provenance.
- Eval gates.
- Tool identity.
- Revocation.
- Human-legible authority.
- Standard cryptography.

Compromise on:

- UI polish early.
- Number of supported tools.
- Number of model providers.
- Number of sandbox lanes.
- Full cloud sync.
- Enterprise features.

## 27. Source Matrix

This source list should be expanded continuously. The initial weighting is primary papers, official specs, official company docs, and mature OS/security references over speculation.

### 27.1 Agent OS And Agent Runtime Papers

- AIOS: LLM Agent Operating System: https://arxiv.org/abs/2403.16971
- OS-Copilot: Towards Generalist Computer Agents with Self-Improvement: https://arxiv.org/abs/2402.07456
- Agent Operating Systems (AOS): Integrating Agentic Control Planes into, and Beyond, Traditional Operating Systems: https://arxiv.org/abs/2606.01508
- Qualixar OS: A Universal Operating System for AI Agent Orchestration: https://arxiv.org/abs/2604.06392
- Toward Securing AI Agents Like Operating Systems: https://arxiv.org/abs/2605.14932
- AgentSentinel: https://arxiv.org/abs/2509.07764
- CaMeLs CUA security work: https://arxiv.org/abs/2601.09923
- CaMeL, Defeating Prompt Injections by Design: https://arxiv.org/abs/2503.18813
- Design Patterns for Securing LLM Agents against Prompt Injections: https://arxiv.org/abs/2506.08837
- FIDES, Securing AI Agents with Information-Flow Control: https://arxiv.org/abs/2505.23643
- Progent, Securing AI Agents with Privilege Control: https://arxiv.org/abs/2504.11703
- Lessons from Defending Gemini Against Indirect Prompt Injections: https://arxiv.org/abs/2505.14534

### 27.2 Benchmarks, Simulations, And Agent Environments

- AgentDojo: https://arxiv.org/abs/2406.13352
- tau-bench (pass^k): https://arxiv.org/abs/2406.12045
- Adding Error Bars to Evals: https://arxiv.org/abs/2411.00640
- OSWorld: https://arxiv.org/abs/2404.07972
- OSWorld 2.0: https://arxiv.org/abs/2606.29537
- WebArena: https://arxiv.org/abs/2307.13854
- BrowserGym: https://arxiv.org/abs/2412.05467
- SWE-bench: https://arxiv.org/abs/2310.06770
- Generative Agents: https://arxiv.org/abs/2304.03442
- Voyager: https://arxiv.org/abs/2305.16291

### 27.3 OS Architecture

- seL4 whitepaper: https://sel4.systems/About/seL4-whitepaper.pdf
- Exokernel paper: https://pdos.csail.mit.edu/6.828/2008/readings/engler95exokernel.pdf
- Singularity research project: https://www.microsoft.com/en-us/research/project/singularity/
- Singularity paper: https://www.microsoft.com/en-us/research/publication/singularity-rethinking-the-software-stack/
- Plan 9 paper: https://9p.io/sys/doc/9.pdf
- WASI: https://wasi.dev/
- WebAssembly core spec: https://webassembly.github.io/spec/core/
- CHERI architecture material: https://www.cl.cam.ac.uk/research/security/ctsrd/cheri/
- OSDev wiki: https://wiki.osdev.org/Main_Page
- OSDev Beginner Mistakes: https://wiki.osdev.org/Beginner_Mistakes
- OSDev Required Knowledge: https://wiki.osdev.org/Required_Knowledge
- Linux sched_ext: https://docs.kernel.org/scheduler/sched-ext.html
- Linux io_uring: https://man7.org/linux/man-pages/man7/io_uring.7.html
- Linux io_uring zero-copy receive: https://docs.kernel.org/networking/iou-zcrx.html
- Linux Rust documentation: https://docs.kernel.org/rust/index.html
- eBPF/XDP program type documentation: https://docs.ebpf.io/linux/program-type/BPF_PROG_TYPE_XDP/
- DPDK poll mode driver documentation: https://doc.dpdk.org/guides/prog_guide/poll_mode_drv.html
- SPDK overview: https://spdk.io/doc/about.html
- Firecracker microVM documentation: https://firecracker-microvm.github.io/
- Linux CXL memory tiering background: https://kernel-internals.org/mm/cxl-memory-tiering/
- NVIDIA CUDA Programming Guide: https://docs.nvidia.com/cuda/cuda-programming-guide/index.html
- NVIDIA CUDA Best Practices Guide: https://docs.nvidia.com/cuda/cuda-c-best-practices-guide/index.html
- NVIDIA Multi-Instance GPU User Guide: https://docs.nvidia.com/datacenter/tesla/mig-user-guide/latest/index.html
- Google Cloud TPU documentation: https://docs.cloud.google.com/tpu/docs
- Google Cloud TPU architecture: https://docs.cloud.google.com/tpu/docs/system-architecture-tpu-vm
- Groq LPU architecture: https://groq.com/lpu-architecture
- Apple Metal documentation: https://developer.apple.com/metal/
- Apple Core ML documentation: https://developer.apple.com/documentation/coreml

### 27.4 Agent Protocols And Company Direction

- Model Context Protocol specification 2025-11-25 (current; supersedes the 2025-06-18 revision cited below): https://modelcontextprotocol.io/specification/2025-11-25
- Model Context Protocol specification 2025-06-18: https://modelcontextprotocol.io/specification/2025-06-18
- MCP authorization specification: https://modelcontextprotocol.io/specification/2025-06-18/basic/authorization
- MCP security best practices: https://modelcontextprotocol.io/specification/2025-06-18/basic/security_best_practices
- Google A2A announcement: https://developers.googleblog.com/en/a2a-a-new-era-of-agent-interoperability/
- OpenAI Operator: https://openai.com/index/introducing-operator/
- OpenAI Computer-Using Agent: https://openai.com/index/computer-using-agent/
- OpenAI ChatGPT agent system card: https://openai.com/index/chatgpt-agent-system-card/
- Anthropic computer use announcement: https://www.anthropic.com/news/3-5-models-and-computer-use
- AWS Bedrock AgentCore: https://aws.amazon.com/blogs/aws/introducing-amazon-bedrock-agentcore-securely-deploy-and-operate-ai-agents-at-any-scale/
- Microsoft Build 2025 agentic web: https://blogs.microsoft.com/blog/2025/05/19/microsoft-build-2025-the-age-of-ai-agents-and-building-the-open-agentic-web/
- Cloudflare Agents docs: https://developers.cloudflare.com/agents/
- NVIDIA Dynamo: https://developer.nvidia.com/blog/introducing-nvidia-dynamo-a-low-latency-distributed-inference-framework-for-scaling-reasoning-ai-models/

### 27.5 Security And Supply Chain

- OWASP Top 10 for LLM Applications: https://owasp.org/www-project-top-10-for-large-language-model-applications/
- Linux seccomp docs: https://www.kernel.org/doc/html/latest/userspace-api/seccomp_filter.html
- Linux cgroups docs: https://www.kernel.org/doc/html/latest/admin-guide/cgroup-v2.html
- AppArmor docs: https://apparmor.net/
- Sigstore: https://www.sigstore.dev/
- Rekor transparency log: https://docs.sigstore.dev/logging/overview/
- SLSA: https://slsa.dev/
- Intel TDX: https://www.intel.com/content/www/us/en/developer/tools/trust-domain-extensions/overview.html
- AMD SEV-SNP: https://www.amd.com/en/developer/sev.html

### 27.7 Authorization, Identity, Policy, And Durable Execution

- Macaroons (NDSS 2014): https://research.google/pubs/macaroons-cookies-with-contextual-caveats-for-decentralized-authorization-in-the-cloud/
- Biscuit: https://www.biscuitsec.org/
- SPIFFE/SPIRE: https://spiffe.io/
- OAuth 2.0 Token Exchange (RFC 8693): https://datatracker.ietf.org/doc/html/rfc8693
- Cedar language paper (OOPSLA 2024): https://arxiv.org/abs/2403.04651
- How We Built Cedar (verification-guided development): https://arxiv.org/abs/2407.01688
- Temporal durable execution: https://temporal.io/
- Restate: https://restate.dev/
- DBOS: https://www.dbos.dev/

### 27.6 Crypto, Payments, And Quantum

- NIST PQC standards announcement: https://www.nist.gov/news-events/news/2024/08/nist-releases-first-3-finalized-post-quantum-encryption-standards
- NIST Post-Quantum Cryptography project: https://csrc.nist.gov/projects/post-quantum-cryptography
- x402: https://x402.org/
- Coinbase x402 docs: https://docs.cdp.coinbase.com/x402/welcome
- AWS on x402 and agentic commerce: https://aws.amazon.com/blogs/industries/x402-and-agentic-commerce-redefining-autonomous-payments-in-financial-services/

## 28. Final Strategic Recommendation

Build beaterOS as a local-first agent operating layer with a minimal trusted agent kernel, and use that hosted kernel to earn the data needed for a future metal-touching OS.

The core invention is not a prettier chatbot, not an immediate Linux replacement, and not a crypto network. The core invention is a set of operating-system-grade contracts for agent work:

- Sessions for goals.
- Capabilities for authority.
- Action manifests for proposed work.
- Policy decisions for admission.
- Receipts for side effects.
- Journals for causality.
- Memory records for provenance.
- Scenarios for evals.

This is the missing layer between probabilistic models and real-world systems.

The design center should be:

- Agent-first.
- Capability-secure.
- Journaled.
- Replayable.
- Eval-gated.
- Sandbox-native.
- Model-agnostic.
- Local-first.
- Payment-aware.
- Crypto-agile.
- Human-legible.
- Metal-ready when evidence demands it.

The first version should make a narrow set of agent workflows dramatically safer and more debuggable than current agent frameworks. If it does that, the project earns the right to become a Linux add-on, a distro, a desktop environment, a cloud runtime, a microVM appliance, and eventually a high-assurance OS research platform whose low-level pieces touch metal for measured reasons.
