# Ubiquitous Language

The shared vocabulary for **RustyBench** — a vendor-independent, pure-Rust suite of
electronics bench tools (logic analyzer, multimeter, oscilloscope, etc.) with a CLI
and an egui GUI that runs natively and in the browser.

## Devices & capabilities

| Term               | Definition                                                                                      | Aliases to avoid             |
| ------------------ | ----------------------------------------------------------------------------------------------- | ---------------------------- |
| **Device**         | A connected piece of instrumentation, modelled at runtime by its set of capabilities            | Instrument, gear, unit       |
| **Driver**         | The code that knows how to talk to a specific device family and expose its capabilities         | Backend, plugin              |
| **Capability**     | A single instrumentation ability a device exposes (e.g. acquire logic, set voltage)             | Feature, function            |
| **Device Class**   | A category of capability: Logic Analyzer, Multimeter, Oscilloscope, Power Supply, Waveform Generator, SDR Receiver, Spectrum Analyzer, Electronic Load | Device type, mode |
| **Multi-class device** | A single device that exposes capabilities from more than one Device Class                    | Combo device, hybrid         |
| **Driver Registry** | The explicit, statically-linked catalogue of available drivers, gated per build target          | Plugin registry              |

## Connectivity

| Term              | Definition                                                                                  | Aliases to avoid           |
| ----------------- | ------------------------------------------------------------------------------------------- | -------------------------- |
| **Transport**     | The byte/packet link a driver speaks over, independent of the concrete medium               | Connection, interface, link, port |
| **Device Candidate** | A reachable-but-not-yet-connected device descriptor (identity + opaque address) produced by a driver scan or manual entry. Lives in `rb-transport`. | Device descriptor, endpoint |
| **Known Device**  | A **Device Candidate** plus its driver name and origin (how we learned about it). The unit the UI displays in the device dropdown. Lives in `rb-core`. | Entry, record, slot |
| **Device Origin** | How a **Known Device** entered the system: `Discovered` (via driver **Scan**) or `Manual` (user added). | Provenance, source |
| **Scan**          | Asking drivers which devices are reachable; actively enumerates on native, prompts the user (browser picker) on web. Produces **Known Devices** with `Discovered` origin. | Discovery, enumerate, probe |
| **Connect**       | Binding a driver to one specific **Device Candidate**, producing a live **Device**. The only way to establish a device connection. | Open, attach               |
| **Disconnect**    | Releasing a live **Device** connection, dropping its handle. The **Known Device** remains in the list (now shown as not connected). Tabs that referenced the device keep their layout and data but can no longer acquire. | Close, detach, release |
| **Additional Info** | Ordered key-value pairs exposed by a connected **Device** beyond basic identity (firmware version, connection type, channel count, …). Returned via `Device::additional_info()`. | Properties, metadata, attributes |
| **Session**       | The central owner of all connected devices for one RustyBench instance                        | Workspace, context         |

## Acquisition & data

| Term                 | Definition                                                                                  | Aliases to avoid              |
| -------------------- | ------------------------------------------------------------------------------------------- | ----------------------------- |
| **Acquisition**      | The act of a device producing a stream of samples into the store                             | Recording, run, sweep         |
| **Capture**          | A stored/loadable acquisition result, including metadata and annotations                     | Recording, file, dump         |
| **Channel**          | One signal source on a device (analog or digital/logic)                                       | Trace, line, pin, wire        |
| **Sample**           | A single measured value on a channel at one point in time                                     | Reading, point, datum         |
| **Sample Rate**      | How many samples per second a channel is acquired at                                          | Speed, frequency              |
| **Timebase**         | The time reference (start time + sample rate) that maps samples to wall-clock time            | Clock, time axis              |
| **Sample Store**     | The per-device structure holding acquired samples plus their multi-resolution aggregation     | Buffer, cache, data store     |
| **Mip-Map**          | The multi-resolution pyramid over a channel (min/max per bucket for analog, edge index for digital) used for display | LOD, pyramid, downsample |

## Decoding

| Term            | Definition                                                                                       | Aliases to avoid          |
| --------------- | ------------------------------------------------------------------------------------------------ | ------------------------- |
| **Decoder**     | A streaming state machine that turns logic channels (or another decoder's output) into annotations | Parser, interpreter       |
| **Stacking**    | Feeding one decoder's output as the input of a higher-level decoder                                | Chaining, layering        |
| **Annotation**  | A typed, time-bounded result emitted by a decoder (e.g. a UART byte, an I²C address)              | Label, marker, tag        |

## Frontends

| Term              | Definition                                                                                                                            | Aliases to avoid          |
| ----------------- | ------------------------------------------------------------------------------------------------------------------------------------- | ------------------------- |
| **CLI**           | The headless, scriptable single-shot tool driving the core without any GUI dependency                                                 | Terminal, console         |
| **Tab**           | A self-contained GUI panel presenting one device or imported capture. Owns a **Tab Source** and **Tab Content**.                      | Panel, window, view       |
| **Tab Source**    | Where a **Tab**'s data comes from: a live **Device**, an imported file, or none (empty tab).                                           | Origin, input             |
| **Tab Content**   | The device-class-specific state inside a **Tab** (e.g. `LogicAnalyzer`, `WaveformGenerator`). Determined once when the source is set. | Tab kind, tab mode        |

## Waveform display

| Term                 | Definition                                                                                       | Aliases to avoid       |
| -------------------- | ------------------------------------------------------------------------------------------------ | ---------------------- |
| **Row**              | The visual representation of one **Channel** (or **Decoder** output) in the waveform canvas. Has a label area, signal area, and optional measurement zones. | Trace row, lane, strip |
| **Row Descriptor**   | A data structure that holds a Row's kind (Analog/Digital/Decoder), height, channel reference, and visibility. | Row config, row spec   |
| **Divider**          | A draggable horizontal separator below each Row that resizes that Row when dragged downward; all Rows below shift with it. | Splitter, resize handle |
| **Signal Area**      | The main waveform drawing region within a Row, showing the **Mip-Map** for the Row's **Channel**. | Wave area, plot area   |
| **Measurement Zone** | A fixed-height strip above (Pulse Width/Peak-to-Peak) and below (Period/Zero-Crossing) the Signal Area; content appears only on mouse hover. | Hover zone, measure bar |
| **Time Ruler**       | A sticky header row at the top of the waveform canvas showing adaptively scaled time tick marks (ns/µs/ms/s) that scroll horizontally with the Rows. | Time axis, time scale  |
| **Marker Bar**        | A sticky strip below the Time Ruler where **Time Markers** are placed and the **Cursor Line** shows the current hover time. | Marker area, marker strip |
| **Time Marker**      | A user-placed reference point at a specific sample position. Can exist standalone or be linked into **Marker Pairs**. Displayed as a draggable diamond/triangle in the Marker Bar. | Flag, pin, bookmark    |
| **Marker Pair**      | Two linked **Time Markers** (A and B) that display Δt and frequency (1/Δt) as a measurement bar between them in the Marker Bar. | Measurement, delta marker |
| **Cursor Line**      | A vertical line spanning all Rows that tracks the mouse position in the Signal Area. Its intersection with the Marker Bar shows the current time (sample offset + time delta). Snaps to digital signal edges when Shift is held. | Cursor, hover line, time cursor |
| **Decoder Row**      | A Row that displays **Annotations** from a specific **Decoder** (referenced by decoder ID). Decoder Rows can be freely interleaved with Analog/Digital Rows. | Protocol row, annotation row |

## Relationships

- A **Session** owns zero or more **Devices** and tracks zero or more **Known Devices**.
- A **Driver** produces **Device Candidates** via **Scan**; a **Device Candidate** plus driver name and origin forms a **Known Device**.
- **Connect** turns a **Known Device** into a live **Device**; **Disconnect** releases it back to a not-connected **Known Device**.
- Each **Capability** corresponds to one **Device Class**; a **Multi-class device** has several.
- A **Driver** talks to its device over a **Transport**; the same driver is unavailable on a platform that lacks its required **Transport**.
- A **Device** performs **Acquisitions** that fill its **Sample Store**; the store maintains a **Mip-Map** per **Channel**.
- A persisted **Acquisition** is a **Capture**.
- A **Decoder** consumes **Channels** (or another **Decoder** via **Stacking**) and emits **Annotations**.
- A **Tab** presents one **Device**'s **Capabilities** and reads from its **Sample Store**; it may also display an imported **Capture** without any live **Device**.
- A **Tab** has a **Tab Source** (Device, File, or empty) and a **Tab Content** that matches one **Device Class**.
- A **Row** visualises a single **Channel** (or a **Decoder** via a **Decoder Row**).
- Each **Row** has a **Divider** that can be dragged to resize the Row; all Rows below shift accordingly.
- A **Decoder Row** references a **Decoder** by ID; multiple Decoder Rows may reference the same Decoder.
- **Time Markers** are placed in the **Marker Bar**; **Marker Pairs** link two markers to compute Δt.
- The **Cursor Line** spans all Rows, following the mouse, and displays the hover time at the **Marker Bar**.

## Example dialogue

> **Dev:** "When the **Session** opens a **Device**, how does a **Tab** know what to show?"

> **Domain expert:** "It asks the **Device** for its **Capabilities**. Each one maps to a **Device Class**. If the device exposes multiple classes, the user picks one — then the **Tab** creates the matching **Tab Content**. For a multi-class device you simply open a second **Tab**."

> **Dev:** "And the waveform itself — does the **Tab** read raw **Samples**?"

> **Domain expert:** "No. It reads the **Mip-Map** in the **Sample Store** at the resolution that fits the current zoom. Raw **Samples** only exist at the bottom level."

> **Dev:** "If I run a **Decoder** on a logic **Channel**, what comes out?"

> **Domain expert:** "**Annotations**. And you can **stack** another **Decoder** on top — for example UART **Annotations** feeding a command-protocol **Decoder**."

> **Dev:** "Does the same **Driver** work in the browser?"

> **Domain expert:** "Only if its **Transport** exists there. A USB **Driver** runs via WebUSB, but a raw-TCP one won't **Scan** at all on web."

## Flagged ambiguities

- **"Channel"** is overloaded: in the domain it is a device's signal source (analog/digital). The internal command/data *channels* (message-passing between GUI and acquisition tasks) are an implementation concept and must **not** be called a Channel in domain conversation.
- **"Acquisition" vs "Capture"**: an **Acquisition** is the live act of producing samples; a **Capture** is the stored, reloadable result. Avoid using "recording" for either.
- **"Device" vs "Driver"**: a **Driver** is the code; a **Device** is the runtime instrumentation it represents. Don't say "driver" when you mean the connected device.
- **"Scan" vs "Connect" vs "Disconnect"**: **Scan** discovers candidates; **Connect** binds to one and produces a live **Device**; **Disconnect** releases it. Connect and Disconnect are the only ways to establish/tear down a device connection.
- **"Known Device" vs "Device"**: a **Known Device** is a descriptor of something we *could* connect to (not yet connected); a **Device** is the live, connected instrumentation. The dropdown shows **Known Devices** split into connected and not-connected sections.
- **"Device Origin"**: `Discovered` means found by a driver **Scan** (replaced on each refresh); `Manual` means hand-entered by the user (survives scans).
- **"Capability" vs "Device Class"**: a **Capability** is what a single device exposes; a **Device Class** is the abstract category. A device *has capabilities*; it *belongs to classes*.
- **"Session"** here always means the RustyBench device session (`rb_core`), never a coding/IDE session.
- **"Tab" vs "Session"**: a **Tab** is a single GUI panel with one **Tab Content**; a **Session** is the central owner of all connected **Devices**. A **Tab** references a **Device** by `DeviceId` but does not own it.
- **"Row" vs "Channel"**: a **Channel** is the device's signal source (domain concept); a **Row** is its visual representation in the waveform canvas (UI concept). Never say "Channel" when you mean the Row that draws it.
- **"Marker"** (time marker) vs **"Annotation"** (decoder annotation): a **Time Marker** is user-placed at an arbitrary sample position; an **Annotation** is automatically emitted by a **Decoder**. They are unrelated concepts.
