# Third-party notices

## Artwork

- **Background art:** by **@kael333_**. The original appears to have been deleted from all platforms or could not be found; only reposts on Pinterest exist. If you are the author and want a link, a different credit or removal, please open an issue.
- **Character:** integrated into the background art with the help of image generation in **ChatGPT**.
- **Icons** (`assets/xp/`, the program icon): generated with an image-generation model for this project.

Using the images from this program outside it is not permitted. See [LICENSE-ART.md](LICENSE-ART.md).

## Fonts

| Font | Where | License |
|---|---|---|
| DejaVu Sans Condensed (regular, bold) | `engine/assets/DejaVuSansCondensed*.ttf` | Bitstream Vera license + public domain DejaVu changes; see `engine/assets/LICENSE-DejaVu.txt` |
| Fusion Pixel, © 2022 TakWolf | `engine/assets/FusionPixel.ttf` | SIL Open Font License 1.1; see `engine/assets/OFL-FusionPixel.txt` |

## Vendored code

- **softbuffer** (`engine/vendor/softbuffer`), © the softbuffer contributors, licensed under MIT OR Apache-2.0. Modified copy of 0.4.8: on macOS an alpha channel, the display colour space and reused frame buffers; size checks instead of panics in the X11, Win32, Wayland and KMS backends. The full diff is `engine/vendor/softbuffer/softbuffer-0.4.8.patch`.

## Bundled C code

- **SQLite** (inside `libsqlite3-sys`, built from source with the `bundled` feature), used read-only to show OpenCode sessions. SQLite is in the public domain.

## Rust crates

These crates are compiled into the binary, depending on the platform. Each is used under the licence shown; the full texts are in each crate's source.

| Crate | Version | License | Source |
|---|---|---|---|
| accesskit | 0.25.2 | MIT OR Apache-2.0 | https://github.com/AccessKit/accesskit |
| accesskit_atspi_common | 0.21.1 | MIT OR Apache-2.0 | https://github.com/AccessKit/accesskit |
| accesskit_consumer | 0.39.2 | MIT OR Apache-2.0 | https://github.com/AccessKit/accesskit |
| accesskit_ios | 0.2.2 | MIT OR Apache-2.0 | https://github.com/AccessKit/accesskit |
| accesskit_macos | 0.27.2 | MIT OR Apache-2.0 | https://github.com/AccessKit/accesskit |
| accesskit_unix | 0.24.1 | MIT OR Apache-2.0 | https://github.com/AccessKit/accesskit |
| accesskit_windows | 0.35.2 | MIT OR Apache-2.0 | https://github.com/AccessKit/accesskit |
| accesskit_winit | 0.34.2 | Apache-2.0 | https://github.com/AccessKit/accesskit |
| adler2 | 2.0.1 | 0BSD OR MIT OR Apache-2.0 | https://github.com/oyvindln/adler2 |
| android-activity | 0.6.1 | MIT OR Apache-2.0 | https://github.com/rust-mobile/android-activity |
| android-properties | 0.2.2 | MIT | https://github.com/miklelappo/android-properties |
| arrayref | 0.3.9 | BSD-2-Clause | https://github.com/droundy/arrayref |
| arrayvec | 0.7.8 | MIT OR Apache-2.0 | https://github.com/bluss/arrayvec |
| as-raw-xcb-connection | 1.0.1 | MIT OR Apache-2.0 | https://github.com/psychon/as-raw-xcb-connection |
| async-broadcast | 0.7.2 | MIT OR Apache-2.0 | https://github.com/smol-rs/async-broadcast |
| async-channel | 2.5.0 | Apache-2.0 OR MIT | https://github.com/smol-rs/async-channel |
| async-executor | 1.14.1 | Apache-2.0 OR MIT | https://github.com/smol-rs/async-executor |
| async-io | 2.6.0 | Apache-2.0 OR MIT | https://github.com/smol-rs/async-io |
| async-lock | 3.5.0 | Apache-2.0 OR MIT | https://github.com/smol-rs/async-lock |
| async-process | 2.5.0 | Apache-2.0 OR MIT | https://github.com/smol-rs/async-process |
| async-recursion | 1.2.0 | MIT OR Apache-2.0 | https://github.com/dcchut/async-recursion |
| async-signal | 0.2.14 | Apache-2.0 OR MIT | https://github.com/smol-rs/async-signal |
| async-task | 4.7.1 | Apache-2.0 OR MIT | https://github.com/smol-rs/async-task |
| async-trait | 0.1.92 | MIT OR Apache-2.0 | https://github.com/dtolnay/async-trait |
| atomic-waker | 1.1.2 | Apache-2.0 OR MIT | https://github.com/smol-rs/atomic-waker |
| atspi | 0.29.0 | Apache-2.0 OR MIT | https://github.com/odilia-app/atspi |
| atspi-common | 0.13.0 | Apache-2.0 OR MIT | https://github.com/odilia-app/atspi |
| atspi-proxies | 0.13.0 | Apache-2.0 OR MIT | https://github.com/odilia-app/atspi |
| autocfg | 1.5.1 | Apache-2.0 OR MIT | https://github.com/cuviper/autocfg |
| base64 | 0.22.1 | MIT OR Apache-2.0 | https://github.com/marshallpierce/rust-base64 |
| bitflags | 1.3.2 | MIT OR Apache-2.0 | https://github.com/bitflags/bitflags |
| bitflags | 2.13.2 | MIT OR Apache-2.0 | https://github.com/bitflags/bitflags |
| block2 | 0.5.1 | MIT | https://github.com/madsmtm/objc2 |
| blocking | 1.7.0 | Apache-2.0 OR MIT | https://github.com/smol-rs/blocking |
| bumpalo | 3.20.3 | MIT OR Apache-2.0 | https://github.com/fitzgen/bumpalo |
| bytemuck | 1.25.2 | Zlib OR Apache-2.0 OR MIT | https://github.com/Lokathor/bytemuck |
| bytemuck_derive | 1.12.1 | Zlib OR Apache-2.0 OR MIT | https://github.com/Lokathor/bytemuck |
| byteorder | 1.5.0 | Unlicense OR MIT | https://github.com/BurntSushi/byteorder |
| bytes | 1.12.1 | MIT | https://github.com/tokio-rs/bytes |
| calloop | 0.13.0 | MIT | https://github.com/Smithay/calloop |
| cc | 1.10.0 | MIT OR Apache-2.0 | https://github.com/rust-lang/cc-rs |
| cfg-if | 1.0.5 | MIT OR Apache-2.0 | https://github.com/rust-lang/cfg-if |
| cfg_aliases | 0.2.2 | MIT | https://github.com/katharostech/cfg_aliases |
| clipboard-win | 5.4.3 | BSL-1.0 | https://github.com/DoumanAsh/clipboard-win |
| combine | 4.6.9 | MIT | https://github.com/Marwes/combine |
| concurrent-queue | 2.5.0 | Apache-2.0 OR MIT | https://github.com/smol-rs/concurrent-queue |
| core-foundation | 0.9.4 | MIT OR Apache-2.0 | https://github.com/servo/core-foundation-rs |
| core-foundation-sys | 0.8.7 | MIT OR Apache-2.0 | https://github.com/servo/core-foundation-rs |
| core-graphics | 0.23.2 | MIT OR Apache-2.0 | https://github.com/servo/core-foundation-rs |
| core-graphics-types | 0.1.3 | MIT OR Apache-2.0 | https://github.com/servo/core-foundation-rs |
| crc32fast | 1.5.3 | MIT OR Apache-2.0 | https://github.com/srijs/rust-crc32fast |
| crossbeam-utils | 0.8.23 | MIT OR Apache-2.0 | https://github.com/crossbeam-rs/crossbeam |
| ctor | 0.10.1 | Apache-2.0 OR MIT | https://github.com/mmastrac/rust-ctor |
| cursor-icon | 1.2.0 | MIT OR Apache-2.0 OR Zlib | https://github.com/rust-windowing/cursor-icon |
| dispatch | 0.2.0 | MIT | http://github.com/SSheldon/rust-dispatch |
| dispatch2 | 0.3.1 | Zlib OR Apache-2.0 OR MIT | https://github.com/madsmtm/objc2 |
| displaydoc | 0.2.7 | MIT OR Apache-2.0 | https://github.com/yaahc/displaydoc |
| dlib | 0.5.3 | MIT | https://github.com/elinorbgr/dlib |
| dpi | 0.1.2 | Apache-2.0 AND MIT | https://github.com/rust-windowing/winit |
| dtor | 0.8.1 | Apache-2.0 OR MIT | https://github.com/mmastrac/rust-ctor |
| endi | 1.1.1 | MIT | https://github.com/zeenix/endi |
| enumflags2 | 0.7.12 | MIT OR Apache-2.0 | https://github.com/meithecatte/enumflags2 |
| enumflags2_derive | 0.7.12 | MIT OR Apache-2.0 | https://github.com/meithecatte/enumflags2 |
| equivalent | 1.0.3 | Apache-2.0 OR MIT | https://github.com/indexmap-rs/equivalent |
| errno | 0.3.14 | MIT OR Apache-2.0 | https://github.com/lambda-fairy/rust-errno |
| error-code | 3.4.0 | BSL-1.0 | https://github.com/DoumanAsh/error-code |
| event-listener | 5.4.2 | Apache-2.0 OR MIT | https://github.com/smol-rs/event-listener |
| event-listener-strategy | 0.5.4 | Apache-2.0 OR MIT | https://github.com/smol-rs/event-listener-strategy |
| fallible-iterator | 0.3.0 | MIT OR Apache-2.0 | https://github.com/sfackler/rust-fallible-iterator |
| fallible-streaming-iterator | 0.1.9 | MIT OR Apache-2.0 | https://github.com/sfackler/fallible-streaming-iterator |
| fastrand | 2.5.0 | Apache-2.0 OR MIT | https://github.com/smol-rs/fastrand |
| fdeflate | 0.3.7 | MIT OR Apache-2.0 | https://github.com/image-rs/fdeflate |
| find-msvc-tools | 0.1.15 | MIT OR Apache-2.0 | https://github.com/rust-lang/cc-rs |
| flate2 | 1.1.10 | MIT OR Apache-2.0 | https://github.com/rust-lang/flate2-rs |
| foldhash | 0.2.0 | Zlib | https://github.com/orlp/foldhash |
| font-types | 0.12.7 | MIT OR Apache-2.0 | https://github.com/googlefonts/fontations |
| foreign-types | 0.5.0 | MIT OR Apache-2.0 | https://github.com/sfackler/foreign-types |
| foreign-types-macros | 0.2.4 | MIT OR Apache-2.0 | https://github.com/sfackler/foreign-types |
| foreign-types-shared | 0.3.1 | MIT OR Apache-2.0 | https://github.com/sfackler/foreign-types |
| form_urlencoded | 1.2.2 | MIT OR Apache-2.0 | https://github.com/servo/rust-url |
| futures-core | 0.3.34 | MIT OR Apache-2.0 | https://github.com/rust-lang/futures-rs |
| futures-io | 0.3.34 | MIT OR Apache-2.0 | https://github.com/rust-lang/futures-rs |
| futures-lite | 2.6.1 | Apache-2.0 OR MIT | https://github.com/smol-rs/futures-lite |
| futures-macro | 0.3.34 | MIT OR Apache-2.0 | https://github.com/rust-lang/futures-rs |
| futures-task | 0.3.34 | MIT OR Apache-2.0 | https://github.com/rust-lang/futures-rs |
| futures-util | 0.3.34 | MIT OR Apache-2.0 | https://github.com/rust-lang/futures-rs |
| gethostname | 1.1.0 | Apache-2.0 | https://codeberg.org/swsnr/gethostname.rs.git |
| getrandom | 0.2.17 | MIT OR Apache-2.0 | https://github.com/rust-random/getrandom |
| getrandom | 0.4.3 | MIT OR Apache-2.0 | https://github.com/rust-random/getrandom |
| hashbrown | 0.17.1 | MIT OR Apache-2.0 | https://github.com/rust-lang/hashbrown |
| hermit-abi | 0.5.3 | MIT OR Apache-2.0 | https://github.com/hermit-os/hermit-rs |
| hex | 0.4.3 | MIT OR Apache-2.0 | https://github.com/KokaKiwi/rust-hex |
| icu_collections | 2.4.0 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| icu_locale_core | 2.4.0 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| icu_normalizer | 2.4.0 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| icu_normalizer_data | 2.4.0 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| icu_properties | 2.4.0 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| icu_properties_data | 2.4.0 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| icu_provider | 2.4.0 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| idna | 1.1.0 | MIT OR Apache-2.0 | https://github.com/servo/rust-url/ |
| idna_adapter | 1.2.2 | Apache-2.0 OR MIT | https://github.com/hsivonen/idna_adapter |
| indexmap | 2.14.2 | Apache-2.0 OR MIT | https://github.com/indexmap-rs/indexmap |
| itoa | 1.0.18 | MIT OR Apache-2.0 | https://github.com/dtolnay/itoa |
| jni | 0.22.4 | MIT OR Apache-2.0 | https://github.com/jni-rs/jni-rs |
| jni-macros | 0.22.4 | MIT OR Apache-2.0 | https://github.com/jni-rs/jni-rs |
| jni-sys | 0.3.1 | MIT OR Apache-2.0 | https://github.com/jni-rs/jni-sys |
| jni-sys | 0.4.1 | MIT OR Apache-2.0 | https://github.com/jni-rs/jni-sys |
| jni-sys-macros | 0.4.1 | MIT OR Apache-2.0 | https://github.com/jni-rs/jni-sys |
| jobserver | 0.1.35 | MIT OR Apache-2.0 | https://github.com/rust-lang/jobserver-rs |
| js-sys | 0.3.106 | MIT OR Apache-2.0 | https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/js-sys |
| libc | 0.2.191 | MIT OR Apache-2.0 | https://github.com/rust-lang/libc |
| libloading | 0.8.9 | ISC | https://github.com/nagisa/rust_libloading/ |
| libredox | 0.1.25 | MIT | https://gitlab.redox-os.org/redox-os/libredox.git |
| libsqlite3-sys | 0.39.1 | MIT | https://github.com/rusqlite/rusqlite |
| linux-raw-sys | 0.12.1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | https://github.com/sunfishcode/linux-raw-sys |
| linux-raw-sys | 0.4.15 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | https://github.com/sunfishcode/linux-raw-sys |
| litemap | 0.8.3 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| log | 0.4.34 | MIT OR Apache-2.0 | https://github.com/rust-lang/log |
| md5 | 0.7.0 | Apache-2.0 OR MIT | https://github.com/stainless-steel/md5 |
| memchr | 2.8.3 | Unlicense OR MIT | https://github.com/BurntSushi/memchr |
| memoffset | 0.9.1 | MIT | https://github.com/Gilnaa/memoffset |
| miniz_oxide | 0.8.9 | MIT OR Zlib OR Apache-2.0 | https://github.com/Frommi/miniz_oxide/tree/master/miniz_oxide |
| miniz_oxide | 0.9.1 | MIT OR Zlib OR Apache-2.0 | https://github.com/Frommi/miniz_oxide/tree/master/miniz_oxide |
| ndk | 0.9.0 | MIT OR Apache-2.0 | https://github.com/rust-mobile/ndk |
| ndk-context | 0.1.1 | MIT OR Apache-2.0 | https://github.com/rust-windowing/android-ndk-rs |
| ndk-sys | 0.6.0+11769913 | MIT OR Apache-2.0 | https://github.com/rust-mobile/ndk |
| num_enum | 0.7.6 | BSD-3-Clause OR MIT OR Apache-2.0 | https://github.com/illicitonion/num_enum |
| num_enum_derive | 0.7.6 | BSD-3-Clause OR MIT OR Apache-2.0 | https://github.com/illicitonion/num_enum |
| objc-sys | 0.3.5 | MIT | https://github.com/madsmtm/objc2 |
| objc2 | 0.5.3 | MIT | https://github.com/madsmtm/objc2 |
| objc2 | 0.6.5 | MIT | https://github.com/madsmtm/objc2 |
| objc2-app-kit | 0.2.2 | MIT | https://github.com/madsmtm/objc2 |
| objc2-cloud-kit | 0.2.2 | MIT | https://github.com/madsmtm/objc2 |
| objc2-contacts | 0.2.2 | MIT | https://github.com/madsmtm/objc2 |
| objc2-core-data | 0.2.2 | MIT | https://github.com/madsmtm/objc2 |
| objc2-core-foundation | 0.3.2 | Zlib OR Apache-2.0 OR MIT | https://github.com/madsmtm/objc2 |
| objc2-core-graphics | 0.3.2 | Zlib OR Apache-2.0 OR MIT | https://github.com/madsmtm/objc2 |
| objc2-core-image | 0.2.2 | MIT | https://github.com/madsmtm/objc2 |
| objc2-core-location | 0.2.2 | MIT | https://github.com/madsmtm/objc2 |
| objc2-encode | 4.1.0 | MIT | https://github.com/madsmtm/objc2 |
| objc2-foundation | 0.2.2 | MIT | https://github.com/madsmtm/objc2 |
| objc2-foundation | 0.3.2 | MIT | https://github.com/madsmtm/objc2 |
| objc2-io-surface | 0.3.2 | Zlib OR Apache-2.0 OR MIT | https://github.com/madsmtm/objc2 |
| objc2-link-presentation | 0.2.2 | MIT | https://github.com/madsmtm/objc2 |
| objc2-metal | 0.2.2 | MIT | https://github.com/madsmtm/objc2 |
| objc2-quartz-core | 0.2.2 | MIT | https://github.com/madsmtm/objc2 |
| objc2-quartz-core | 0.3.2 | Zlib OR Apache-2.0 OR MIT | https://github.com/madsmtm/objc2 |
| objc2-symbols | 0.2.2 | MIT | https://github.com/madsmtm/objc2 |
| objc2-ui-kit | 0.2.2 | MIT | https://github.com/madsmtm/objc2 |
| objc2-uniform-type-identifiers | 0.2.2 | MIT | https://github.com/madsmtm/objc2 |
| objc2-user-notifications | 0.2.2 | MIT | https://github.com/madsmtm/objc2 |
| once_cell | 1.21.4 | MIT OR Apache-2.0 | https://github.com/matklad/once_cell |
| orbclient | 0.3.55 | MIT | https://gitlab.redox-os.org/redox-os/orbclient |
| ordered-stream | 0.2.0 | MIT OR Apache-2.0 | https://github.com/danieldg/ordered-stream |
| parking | 2.2.1 | Apache-2.0 OR MIT | https://github.com/smol-rs/parking |
| percent-encoding | 2.3.2 | MIT OR Apache-2.0 | https://github.com/servo/rust-url/ |
| phf | 0.14.0 | MIT | https://github.com/rust-phf/rust-phf |
| phf_generator | 0.14.0 | MIT | https://github.com/rust-phf/rust-phf |
| phf_macros | 0.14.0 | MIT | https://github.com/rust-phf/rust-phf |
| phf_shared | 0.14.0 | MIT | https://github.com/rust-phf/rust-phf |
| pin-project | 1.1.13 | Apache-2.0 OR MIT | https://github.com/taiki-e/pin-project |
| pin-project-internal | 1.1.13 | Apache-2.0 OR MIT | https://github.com/taiki-e/pin-project |
| pin-project-lite | 0.2.17 | Apache-2.0 OR MIT | https://github.com/taiki-e/pin-project-lite |
| piper | 0.2.5 | MIT OR Apache-2.0 | https://github.com/smol-rs/piper |
| pkg-config | 0.3.34 | MIT OR Apache-2.0 | https://github.com/rust-lang/pkg-config-rs |
| plain | 0.2.3 | MIT OR Apache-2.0 | https://github.com/randomites/plain |
| png | 0.17.16 | MIT OR Apache-2.0 | https://github.com/image-rs/image-png |
| polling | 3.11.0 | Apache-2.0 OR MIT | https://github.com/smol-rs/polling |
| potential_utf | 0.1.6 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| proc-macro-crate | 3.5.0 | MIT OR Apache-2.0 | https://github.com/bkchr/proc-macro-crate |
| proc-macro2 | 1.0.107 | MIT OR Apache-2.0 | https://github.com/dtolnay/proc-macro2 |
| quote | 1.0.47 | MIT OR Apache-2.0 | https://github.com/dtolnay/quote |
| r-efi | 6.0.0 | MIT OR Apache-2.0 OR LGPL-2.1-or-later | https://github.com/r-efi/r-efi |
| raw-window-handle | 0.6.2 | MIT OR Apache-2.0 OR Zlib | https://github.com/rust-windowing/raw-window-handle |
| read-fonts | 0.46.0 | MIT OR Apache-2.0 | https://github.com/googlefonts/fontations |
| redox_syscall | 0.4.1 | MIT | https://gitlab.redox-os.org/redox-os/syscall |
| redox_syscall | 0.5.18 | MIT | https://gitlab.redox-os.org/redox-os/syscall |
| redox_syscall | 0.9.5 | MIT | https://gitlab.redox-os.org/redox-os/kernel |
| ring | 0.17.14 | Apache-2.0 AND ISC | https://github.com/briansmith/ring |
| rusqlite | 0.41.1 | MIT | https://github.com/rusqlite/rusqlite |
| rustc_version | 0.4.1 | MIT OR Apache-2.0 | https://github.com/djc/rustc-version-rs |
| rustix | 0.38.44 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | https://github.com/bytecodealliance/rustix |
| rustix | 1.1.6 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | https://github.com/bytecodealliance/rustix |
| rustls | 0.23.46 | Apache-2.0 OR ISC OR MIT | https://github.com/rustls/rustls |
| rustls-pki-types | 1.15.1 | MIT OR Apache-2.0 | https://github.com/rustls/pki-types |
| rustls-webpki | 0.103.16 | ISC | https://github.com/rustls/webpki |
| rustversion | 1.0.23 | MIT OR Apache-2.0 | https://github.com/dtolnay/rustversion |
| same-file | 1.0.6 | Unlicense OR MIT | https://github.com/BurntSushi/same-file |
| semver | 1.0.28 | MIT OR Apache-2.0 | https://github.com/dtolnay/semver |
| serde | 1.0.229 | MIT OR Apache-2.0 | https://github.com/serde-rs/serde |
| serde_core | 1.0.229 | MIT OR Apache-2.0 | https://github.com/serde-rs/serde |
| serde_derive | 1.0.229 | MIT OR Apache-2.0 | https://github.com/serde-rs/serde |
| serde_json | 1.0.154 | MIT OR Apache-2.0 | https://github.com/serde-rs/json |
| serde_repr | 0.1.21 | MIT OR Apache-2.0 | https://github.com/dtolnay/serde-repr |
| shlex | 2.0.1 | MIT OR Apache-2.0 | https://github.com/comex/rust-shlex |
| signal-hook-registry | 1.4.8 | MIT OR Apache-2.0 | https://github.com/vorner/signal-hook |
| simd-adler32 | 0.3.10 | MIT | https://github.com/mcountryman/simd-adler32 |
| simd_cesu8 | 1.2.0 | Apache-2.0 OR MIT | https://github.com/seancroach/simd_cesu8 |
| simdutf8 | 0.1.5 | MIT OR Apache-2.0 | https://github.com/rusticstuff/simdutf8 |
| siphasher | 1.0.4 | MIT OR Apache-2.0 | https://github.com/jedisct1/rust-siphash |
| skrifa | 0.49.0 | MIT OR Apache-2.0 | https://github.com/googlefonts/fontations |
| slab | 0.4.12 | MIT | https://github.com/tokio-rs/slab |
| smallvec | 1.16.4 | MIT OR Apache-2.0 | https://github.com/servo/rust-smallvec |
| smol_str | 0.2.2 | MIT OR Apache-2.0 | https://github.com/rust-analyzer/smol_str |
| socks | 0.3.4 | MIT OR Apache-2.0 | https://github.com/sfackler/rust-socks |
| stable_deref_trait | 1.2.1 | MIT OR Apache-2.0 | https://github.com/storyyeller/stable_deref_trait |
| static_assertions | 1.1.0 | MIT OR Apache-2.0 | https://github.com/nvzqz/static-assertions-rs |
| strict-num | 0.1.1 | MIT | https://github.com/RazrFalcon/strict-num |
| subtle | 2.6.1 | BSD-3-Clause | https://github.com/dalek-cryptography/subtle |
| syn | 2.0.119 | MIT OR Apache-2.0 | https://github.com/dtolnay/syn |
| syn | 3.0.9 | MIT OR Apache-2.0 | https://github.com/dtolnay/syn |
| synstructure | 0.14.0 | MIT | https://github.com/mystor/synstructure |
| tempfile | 3.27.0 | MIT OR Apache-2.0 | https://github.com/Stebalien/tempfile |
| thiserror | 1.0.69 | MIT OR Apache-2.0 | https://github.com/dtolnay/thiserror |
| thiserror | 2.0.21 | MIT OR Apache-2.0 | https://github.com/dtolnay/thiserror |
| thiserror-impl | 1.0.69 | MIT OR Apache-2.0 | https://github.com/dtolnay/thiserror |
| thiserror-impl | 2.0.21 | MIT OR Apache-2.0 | https://github.com/dtolnay/thiserror |
| tiny-skia | 0.11.4 | BSD-3-Clause | https://github.com/RazrFalcon/tiny-skia |
| tiny-skia-path | 0.11.4 | BSD-3-Clause | https://github.com/RazrFalcon/tiny-skia/tree/master/path |
| tiny-xlib | 0.2.5 | MIT OR Apache-2.0 OR Zlib | https://github.com/rust-windowing/tiny-xlib |
| tinystr | 0.8.5 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| tokio | 1.54.1 | MIT | https://github.com/tokio-rs/tokio |
| toml_datetime | 1.1.2+spec-1.1.0 | MIT OR Apache-2.0 | https://github.com/toml-rs/toml |
| toml_edit | 0.25.17+spec-1.1.0 | MIT OR Apache-2.0 | https://github.com/toml-rs/toml |
| toml_parser | 1.1.5+spec-1.1.0 | MIT OR Apache-2.0 | https://github.com/toml-rs/toml |
| tracing | 0.1.44 | MIT | https://github.com/tokio-rs/tracing |
| tracing-attributes | 0.1.31 | MIT | https://github.com/tokio-rs/tracing |
| tracing-core | 0.1.36 | MIT | https://github.com/tokio-rs/tracing |
| uds_windows | 1.2.1 | MIT | https://github.com/haraldh/rust_uds_windows |
| unicode-ident | 1.0.27 | (MIT OR Apache-2.0) AND Unicode-3.0 | https://github.com/dtolnay/unicode-ident |
| unicode-segmentation | 1.13.3 | MIT OR Apache-2.0 | https://github.com/unicode-rs/unicode-segmentation |
| untrusted | 0.9.0 | ISC | https://github.com/briansmith/untrusted |
| ureq | 2.12.1 | MIT OR Apache-2.0 | https://github.com/algesten/ureq |
| url | 2.5.8 | MIT OR Apache-2.0 | https://github.com/servo/rust-url |
| utf8_iter | 1.1.0 | Apache-2.0 OR MIT | https://github.com/hsivonen/utf8_iter |
| utf_types | 1.0.0 | Apache-2.0 OR MIT | https://github.com/hsivonen/utf_types |
| uuid | 1.29.0 | Apache-2.0 OR MIT | https://github.com/uuid-rs/uuid |
| vcpkg | 0.2.15 | MIT OR Apache-2.0 | https://github.com/mcgoo/vcpkg-rs |
| walkdir | 2.5.0 | Unlicense OR MIT | https://github.com/BurntSushi/walkdir |
| wasi | 0.11.1+wasi-snapshot-preview1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | https://github.com/bytecodealliance/wasi |
| wasm-bindgen | 0.2.129 | MIT OR Apache-2.0 | https://github.com/wasm-bindgen/wasm-bindgen |
| wasm-bindgen-futures | 0.4.79 | MIT OR Apache-2.0 | https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/futures |
| wasm-bindgen-macro | 0.2.129 | MIT OR Apache-2.0 | https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/macro |
| wasm-bindgen-macro-support | 0.2.129 | MIT OR Apache-2.0 | https://github.com/wasm-bindgen/wasm-bindgen/tree/main/crates/macro-support |
| wasm-bindgen-shared | 0.2.129 | MIT OR Apache-2.0 | https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/shared |
| web-sys | 0.3.106 | MIT OR Apache-2.0 | https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/web-sys |
| web-time | 1.1.0 | MIT OR Apache-2.0 | https://github.com/daxpedda/web-time |
| webpki-roots | 0.26.11 | CDLA-Permissive-2.0 | https://github.com/rustls/webpki-roots |
| webpki-roots | 1.0.11 | CDLA-Permissive-2.0 | https://github.com/rustls/webpki-roots |
| winapi | 0.3.9 | MIT OR Apache-2.0 | https://github.com/retep998/winapi-rs |
| winapi-i686-pc-windows-gnu | 0.4.0 | MIT OR Apache-2.0 | https://github.com/retep998/winapi-rs |
| winapi-util | 0.1.11 | Unlicense OR MIT | https://github.com/BurntSushi/winapi-util |
| winapi-x86_64-pc-windows-gnu | 0.4.0 | MIT OR Apache-2.0 | https://github.com/retep998/winapi-rs |
| windows | 0.62.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-collections | 0.3.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-core | 0.62.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-future | 0.3.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-implement | 0.60.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-interface | 0.59.3 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-link | 0.2.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-numerics | 0.3.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-result | 0.4.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-strings | 0.5.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-sys | 0.52.0 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-sys | 0.59.0 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-sys | 0.61.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-targets | 0.52.6 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-threading | 0.2.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows_aarch64_gnullvm | 0.52.6 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows_aarch64_msvc | 0.52.6 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows_i686_gnu | 0.52.6 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows_i686_gnullvm | 0.52.6 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows_i686_msvc | 0.52.6 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows_x86_64_gnu | 0.52.6 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows_x86_64_gnullvm | 0.52.6 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows_x86_64_msvc | 0.52.6 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| winit | 0.30.13 | Apache-2.0 | https://github.com/rust-windowing/winit |
| winnow | 1.0.4 | MIT | https://github.com/winnow-rs/winnow |
| writeable | 0.6.4 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| x11-dl | 2.21.0 | MIT | https://github.com/AltF02/x11-rs.git |
| x11rb | 0.13.2 | MIT OR Apache-2.0 | https://github.com/psychon/x11rb |
| x11rb-protocol | 0.13.2 | MIT OR Apache-2.0 | https://github.com/psychon/x11rb |
| xkbcommon-dl | 0.4.2 | MIT | https://github.com/rust-windowing/xkbcommon-dl |
| xkeysym | 0.2.1 | MIT OR Apache-2.0 OR Zlib | https://github.com/notgull/xkeysym |
| yoke | 0.8.4 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| yoke-derive | 0.8.4 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| zbus | 5.19.0 | MIT | https://github.com/z-galaxy/zbus/ |
| zbus-lockstep | 0.5.2 | MIT | https://github.com/luukvanderduim/zbus-lockstep |
| zbus-lockstep-macros | 0.5.2 | MIT | https://github.com/luukvanderduim/zbus-lockstep |
| zbus_macros | 5.19.0 | MIT | https://github.com/z-galaxy/zbus/ |
| zbus_names | 4.3.5 | MIT | https://github.com/z-galaxy/zbus/ |
| zbus_xml | 5.2.1 | MIT | https://github.com/z-galaxy/zbus/ |
| zcheapstr | 1.2.0 | MIT | https://github.com/z-galaxy/zcheapstr/ |
| zerofrom | 0.1.8 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| zerofrom-derive | 0.1.8 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| zeroize | 1.9.1 | Apache-2.0 OR MIT | https://github.com/RustCrypto/utils |
| zerotrie | 0.2.5 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| zerovec | 0.11.9 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| zerovec-derive | 0.11.7 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| zlib-rs | 0.6.8 | Zlib | https://github.com/trifectatechfoundation/zlib-rs |
| zmij | 1.0.23 | MIT | https://github.com/dtolnay/zmij |
| zvariant | 5.15.1 | MIT | https://github.com/z-galaxy/zbus/ |
| zvariant_derive | 5.15.1 | MIT | https://github.com/z-galaxy/zbus/ |
| zvariant_utils | 4.2.0 | MIT | https://github.com/z-galaxy/zbus/ |
