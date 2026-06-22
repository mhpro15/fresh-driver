# Third-party notices

Fresh Driver bundles or builds on the following third-party components.

## pci.ids (bundled — `src-tauri/data/pci.ids`)

The PCI ID database from the PCI ID Repository (https://pci-ids.ucw.cz),
maintained by Martin Mares and contributors. Used here under the **3-clause BSD
license** (the database is dual-licensed BSD-3-Clause OR GNU GPL v2-or-later).
The full license text is included at the top of the bundled `pci.ids` file.

Used to display human-readable vendor and device names. Not modified.

## Runtime data sources (not bundled — queried at runtime)

- **Microsoft Update Catalog** / **Windows Update Agent** — Microsoft.
- **NVIDIA** driver lookup (`gfwsl.geforce.com`) and the community GPU↔ID map
  (github.com/ZenitH-AT/nvidia-data) — used only to look up the latest published
  driver version and link to NVIDIA's official download. No files are rehosted.
- **Dell / HP / Lenovo** official catalogs and tools, used only on the matching
  hardware brand. Downloads always come from the vendor's own servers.

## Frameworks

Built with **Tauri** (MIT / Apache-2.0) and the Rust ecosystem. See each crate's
license in the dependency tree (`cargo` metadata) for details.
