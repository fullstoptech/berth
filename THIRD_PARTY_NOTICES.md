# Third-party notices

Berth is released under `MIT OR Apache-2.0` (`LICENSE-MIT`, `LICENSE-APACHE`). It ships with the software below, each under its own licence.

This file is generated: `python3 -I scripts/third-party-notices.py > THIRD_PARTY_NOTICES.md`. Do not edit it by hand.

## Fonts

Embedded in the application binary (`crates/berth-app/assets/fonts/`), under the SIL Open Font License 1.1. The notice and licence below are the file `OFL.txt` shipped with the fonts.

| Font | Files |
|---|---|
| IBM Plex Sans | `IBMPlexSans-400.ttf`, `-500.ttf`, `-600.ttf` |
| JetBrains Mono | `JetBrainsMono-400.ttf`, `-500.ttf` |

```text
Copyright 2019 IBM Corp. All rights reserved. (IBM Plex Sans)
Copyright 2020 The JetBrains Mono Project Authors (https://github.com/JetBrains/JetBrainsMono) (JetBrains Mono)

This Font Software is licensed under the SIL Open Font License,
Version 1.1.

This license is copied below, and is also available with a FAQ at:
http://scripts.sil.org/OFL

-----------------------------------------------------------
SIL OPEN FONT LICENSE Version 1.1 - 26 February 2007
-----------------------------------------------------------

PREAMBLE
The goals of the Open Font License (OFL) are to stimulate worldwide
development of collaborative font projects, to support the font
creation efforts of academic and linguistic communities, and to
provide a free and open framework in which fonts may be shared and
improved in partnership with others.

The OFL allows the licensed fonts to be used, studied, modified and
redistributed freely as long as they are not sold by themselves. The
fonts, including any derivative works, can be bundled, embedded,
redistributed and/or sold with any software provided that any reserved
names are not used by derivative works. The fonts and derivatives,
however, cannot be released under any other type of license. The
requirement for fonts to remain under this license does not apply to
any document created using the fonts or their derivatives.

DEFINITIONS
"Font Software" refers to the set of files released by the Copyright
Holder(s) under this license and clearly marked as such. This may
include source files, build scripts and documentation.

"Reserved Font Name" refers to any names specified as such after the
copyright statement(s).

"Original Version" refers to the collection of Font Software
components as distributed by the Copyright Holder(s).

"Modified Version" refers to any derivative made by adding to,
deleting, or substituting -- in part or in whole -- any of the
components of the Original Version, by changing formats or by porting
the Font Software to a new environment.

"Author" refers to any designer, engineer, programmer, technical
writer or other person who contributed to the Font Software.

PERMISSION & CONDITIONS
Permission is hereby granted, free of charge, to any person obtaining
a copy of the Font Software, to use, study, copy, merge, embed,
modify, redistribute, and sell modified and unmodified copies of the
Font Software, subject to the following conditions:

1) Neither the Font Software nor any of its individual components, in
Original or Modified Versions, may be sold by itself.

2) Original or Modified Versions of the Font Software may be bundled,
redistributed and/or sold with any software, provided that each copy
contains the above copyright notice and this license. These can be
included either as stand-alone text files, human-readable headers or
in the appropriate machine-readable metadata fields within text or
binary files as long as those fields can be easily viewed by the user.

3) No Modified Version of the Font Software may use the Reserved Font
Name(s) unless explicit written permission is granted by the
corresponding Copyright Holder. This restriction only applies to the
primary font name as presented to the users.

4) The name(s) of the Copyright Holder(s) or the Author(s) of the Font
Software shall not be used to promote, endorse or advertise any
Modified Version, except to acknowledge the contribution(s) of the
Copyright Holder(s) and the Author(s) or with their explicit written
permission.

5) The Font Software, modified or unmodified, in part or in whole,
must be distributed entirely under this license, and must not be
distributed under any other license. The requirement for fonts to
remain under this license does not apply to any document created using
the Font Software.

TERMINATION
This license becomes null and void if any of the above conditions are
not met.

DISCLAIMER
THE FONT SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO ANY WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT
OF COPYRIGHT, PATENT, TRADEMARK, OR OTHER RIGHT. IN NO EVENT SHALL THE
COPYRIGHT HOLDER BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY,
INCLUDING ANY GENERAL, SPECIAL, INDIRECT, INCIDENTAL, OR CONSEQUENTIAL
DAMAGES, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
FROM, OUT OF THE USE OR INABILITY TO USE THE FONT SOFTWARE OR FROM
OTHER DEALINGS IN THE FONT SOFTWARE.
```

## Terminal and editor bundles (npm)

`assets/web/terminal.js` and `assets/web/editor.js` are built by `scripts/build-web.sh` from `web-src/` (versions pinned in `web-src/pnpm-lock.yaml`). Every package below is MIT licensed under the copyright line shown; the licence text follows the table.

| Package | Version | Copyright |
|---|---|---|
| @codemirror/autocomplete | 6.20.4 | Copyright (C) 2018-2021 by Marijn Haverbeke <marijn@haverbeke.berlin> and others |
| @codemirror/commands | 6.11.1 | Copyright (C) 2018-2021 by Marijn Haverbeke <marijn@haverbeke.berlin> and others |
| @codemirror/lang-yaml | 6.1.3 | Copyright (C) 2024 by Marijn Haverbeke <marijn@haverbeke.berlin> and others |
| @codemirror/language | 6.13.1 | Copyright (C) 2018-2021 by Marijn Haverbeke <marijn@haverbeke.berlin> and others |
| @codemirror/lint | 6.9.7 | Copyright (C) 2018-2021 by Marijn Haverbeke <marijn@haverbeke.berlin> and others |
| @codemirror/search | 6.7.2 | Copyright (C) 2018-2021 by Marijn Haverbeke <marijn@haverbeke.berlin> and others |
| @codemirror/state | 6.7.6 | Copyright (C) 2018-2021 by Marijn Haverbeke <marijn@haverbeke.berlin> and others |
| @codemirror/streamparser | 6.0.0 | Copyright (C) 2018-2021 by Marijn Haverbeke <marijn@haverbeke.berlin> and others |
| @codemirror/view | 6.43.15 | Copyright (C) 2018-2021 by Marijn Haverbeke <marijn@haverbeke.berlin> and others |
| @lezer/common | 1.5.4 | Copyright (C) 2018 by Marijn Haverbeke <marijn@haverbeke.berlin> and others |
| @lezer/highlight | 1.2.5 | Copyright (C) 2018 by Marijn Haverbeke <marijn@haverbeke.berlin> and others |
| @lezer/lr | 1.4.11 | Copyright (C) 2018 by Marijn Haverbeke <marijn@haverbeke.berlin> and others |
| @lezer/yaml | 1.0.4 | Copyright (C) 2024 by Marijn Haverbeke <marijnh@gmail.com> and others |
| @marijn/find-cluster-break | 1.0.4 | Copyright (C) 2024 by Marijn Haverbeke <marijn@haverbeke.berlin> |
| @xterm/addon-fit | 0.11.0 | Copyright (c) 2019, The xterm.js authors (https://github.com/xtermjs/xterm.js) |
| @xterm/addon-search | 0.16.0 | Copyright (c) 2017, The xterm.js authors (https://github.com/xtermjs/xterm.js) |
| @xterm/addon-web-links | 0.12.0 | Copyright (c) 2017, The xterm.js authors (https://github.com/xtermjs/xterm.js) |
| @xterm/xterm | 6.0.0 | Copyright (c) 2017-2019, The xterm.js authors (https://github.com/xtermjs/xterm.js) |
| codemirror | 6.0.2 | Copyright (C) 2018-2021 by Marijn Haverbeke <marijn@haverbeke.berlin> and others |
| crelt | 1.0.7 | Copyright (C) 2020 by Marijn Haverbeke <marijn@haverbeke.berlin> |
| style-mod | 4.1.4 | Copyright (C) 2018 by Marijn Haverbeke <marijn@haverbeke.berlin> and others |
| w3c-keyname | 2.2.8 | Copyright (C) 2016 by Marijn Haverbeke <marijn@haverbeke.berlin> and others |

```text
Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Rust crates

416 crates are compiled into `berth` and `berth-coolify` for `aarch64-apple-darwin` (normal dependencies of both, from `cargo metadata`; build-only, dev-only and other-platform crates are left out). Grouped by licence expression. Where an expression offers a choice (`OR`), Berth takes the terms of any one alternative. Each crate's full licence text and copyright notices are in its published source package on crates.io.

### Apache-2.0 OR MIT (252)

aligned 0.4.3, anyhow 1.0.104, arrayvec 0.7.8, as-slice 0.2.1, askama_escape 0.13.0, async-trait 0.1.92, atomic-waker 1.1.2, base64 0.22.1, base64 0.23.1, bit_field 0.10.3, bitflags 1.3.2, bitflags 2.13.2, bitstream-io 4.10.0, block-buffer 0.10.4, block-buffer 0.12.1, bstr 1.13.1, bumpalo 3.20.3, cfg-if 1.0.5, chrono 0.4.45, cocoa 0.26.1, cocoa-foundation 0.2.1, const-oid 0.10.2, const-serialize 0.7.2, const-serialize 0.8.0-alpha.0, const-serialize-macro 0.7.2, const-serialize-macro 0.8.0-alpha.3, cookie 0.18.2, core-foundation 0.10.1, core-foundation-sys 0.8.7, core-graphics 0.24.0, core-graphics 0.25.0, core-graphics-types 0.2.0, cpufeatures 0.2.17, cpufeatures 0.3.1, crc32fast 1.5.3, crossbeam-channel 0.5.17, crossbeam-deque 0.8.8, crossbeam-epoch 0.9.21, crossbeam-utils 0.8.23, crypto-common 0.1.7, crypto-common 0.2.2, deranged 0.5.9, digest 0.10.7, digest 0.11.3, dioxus 0.7.10, dioxus-asset-resolver 0.7.10, dioxus-cli-config 0.7.10, dioxus-config-macro 0.7.10, dioxus-config-macros 0.7.10, dioxus-core 0.7.10, dioxus-core-macro 0.7.10, dioxus-core-types 0.7.10, dioxus-desktop 0.7.10, dioxus-devtools 0.7.10, dioxus-devtools-types 0.7.10, dioxus-document 0.7.10, dioxus-history 0.7.10, dioxus-hooks 0.7.10, dioxus-html 0.7.10, dioxus-html-internal-macro 0.7.10, dioxus-interpreter-js 0.7.10, dioxus-rsx 0.7.10, dioxus-signals 0.7.10, dioxus-ssr 0.7.10, dioxus-stores 0.7.10, dioxus-stores-macro 0.7.10, dioxus-web 0.7.10, directories 6.0.0, dirs 6.0.0, dirs-sys 0.5.0, displaydoc 0.2.7, dyn-clone 1.0.20, either 1.19.0, enumset 1.1.15, enumset_derive 0.15.1, equivalent 1.0.3, errno 0.3.14, euclid 0.22.14, fallible-iterator 0.3.0, fallible-streaming-iterator 0.1.9, fastrand 2.5.0, fdeflate 0.3.7, filetime 0.2.29, flate2 1.1.10, fnv 1.0.7, foreign-types 0.5.0, foreign-types-macros 0.2.4, foreign-types-shared 0.3.1, form_urlencoded 1.2.2, futures 0.3.34, futures-channel 0.3.34, futures-core 0.3.34, futures-executor 0.3.34, futures-io 0.3.34, futures-macro 0.3.34, futures-sink 0.3.34, futures-task 0.3.34, futures-util 0.3.34, generational-box 0.7.10, getrandom 0.2.17, getrandom 0.3.4, getrandom 0.4.3, gif 0.14.2, global-hotkey 0.7.0, gloo-timers 0.3.0, half 2.7.1, hashbrown 0.17.1, hashlink 0.12.2, hex 0.4.3, home 0.5.12, http 1.5.0, httparse 1.10.1, httpdate 1.0.3, hybrid-array 0.4.15, iana-time-zone 0.1.65, ident_case 1.0.1, idna 1.1.0, idna_adapter 1.2.2, image 0.25.10, image-webp 0.2.4, indexmap 2.14.2, ipnet 2.12.2, itertools 0.14.0, itoa 1.0.18, js-sys 0.3.106, keyboard-types 0.7.0, keyring 3.6.3, lazy_static 1.5.1, libc 0.2.191, lock_api 0.4.14, log 0.4.34, longest-increasing-subsequence 0.1.0, macro-string 0.1.4, manganis 0.7.10, manganis-core 0.7.10, manganis-macro 0.7.10, memmap2 0.9.12, muda 0.17.2, native-tls 0.2.18, no_std_io2 0.9.4, num-bigint 0.4.8, num-complex 0.4.6, num-conv 0.2.2, num-derive 0.4.2, num-integer 0.1.47, num-rational 0.4.2, num-traits 0.2.19, once_cell 1.21.4, openssh 0.11.6, parking_lot 0.12.5, parking_lot_core 0.9.12, paste 1.0.15, pastey 0.1.1, pastey 0.2.3, percent-encoding 2.3.2, pin-project 1.1.13, pin-project-internal 1.1.13, pin-project-lite 0.2.17, png 0.17.16, png 0.18.1, powerfmt 0.2.1, ppv-lite86 0.2.21, proc-macro2 1.0.107, proc-macro2-diagnostics 0.10.1, process-wrap 10.0.1, profiling 1.0.18, profiling-procmacros 1.0.18, qoi 0.4.1, quick-error 2.0.1, quote 1.0.47, rand 0.9.5, rand_chacha 0.9.0, rand_core 0.9.5, rayon 1.12.0, rayon-core 1.13.0, ref-cast 1.0.27, ref-cast-impl 1.0.27, regex 1.13.1, regex-automata 0.4.18, regex-syntax 0.8.11, reqwest 0.13.5, rustc-hash 1.1.0, rustc-hash 2.1.3, rustls-pki-types 1.15.1, rustls-platform-verifier 0.7.1, rustversion 1.0.23, scopeguard 1.2.0, secrecy 0.10.3, security-framework 3.7.0, security-framework-sys 2.17.0, semver 1.0.28, send_wrapper 0.6.0, serde 1.0.229, serde_core 1.0.229, serde_derive 1.0.229, serde_derive_internals 0.30.0, serde_json 1.0.154, serde_norway 0.9.42, serde_repr 0.1.21, serde_spanned 1.1.2, serde_urlencoded 0.7.1, sha1 0.10.7, sha2 0.11.1, shell-escape 0.1.5, shell-words 1.1.1, signal-hook 0.3.18, signal-hook-registry 1.4.8, smallvec 1.16.4, socket2 0.6.5, stable_deref_trait 1.2.1, strip-ansi-escapes 0.2.1, subsecond 0.7.10, subsecond-types 0.7.10, syn 2.0.119, syn 3.0.9, tar 0.4.46, tempfile 3.27.0, thiserror 2.0.21, thiserror-impl 2.0.21, thread_local 1.1.10, time 0.3.55, time-core 0.1.9, time-macros 0.2.32, tokio-rustls 0.26.6, toml 1.1.8+spec-1.1.0, toml_datetime 1.1.2+spec-1.1.0, toml_parser 1.1.5+spec-1.1.0, toml_writer 1.1.3+spec-1.1.0, tray-icon 0.21.3, tungstenite 0.28.0, typenum 1.20.1, unicode-segmentation 1.13.3, unicode-xid 0.2.6, url 2.5.8, utf-8 0.7.6, utf8_iter 1.0.4, uuid 1.29.0, vte 0.14.1, warnings 0.2.1, warnings-macro 0.2.0, wasm-bindgen 0.2.129, wasm-bindgen-futures 0.4.79, wasm-bindgen-macro 0.2.129, wasm-bindgen-macro-support 0.2.129, wasm-bindgen-shared 0.2.129, wasm-streams 0.4.2, web-sys 0.3.106, webbrowser 1.2.5, weezl 0.1.12, wry 0.53.5, xattr 1.6.1, zeroize 1.9.1

### MIT (85)

aligned-vec 0.6.4, arg_enum_proc_macro 0.3.4, av-scenechange 0.14.1, block 0.1.6, block2 0.6.2, bytes 1.12.1, cfb 0.7.3, color_quant 1.1.0, convert_case 0.8.0, darling 0.21.3, darling 0.24.1, darling_core 0.21.3, darling_core 0.24.1, darling_macro 0.21.3, darling_macro 0.24.1, data-encoding 2.11.1, dioxus-logger 0.7.10, equator 0.4.2, equator-macro 0.4.2, fax 0.2.7, generic-array 0.14.7, h2 0.4.20, http-body 1.1.0, http-body-util 0.1.5, hyper 1.12.0, hyper-util 0.1.21, hyperlocal 0.9.1, infer 0.19.0, libm 0.2.16, libsqlite3-sys 0.39.0, loop9 0.1.5, malloc_buf 0.0.6, matchers 0.2.0, maybe-rayon 0.1.1, mio 1.2.4, new_debug_unreachable 1.0.6, nix 0.31.3, nom 8.0.0, noop_proc_macro 0.3.0, nu-ansi-term 0.50.3, objc 0.2.7, objc2 0.6.5, objc2-encode 4.1.0, objc2-foundation 0.3.2, pulp 0.22.3, pulp-wasm-simd-flag 0.1.1, reborrow 0.5.5, rfd 0.17.2, rgb 0.8.53, rusqlite 0.41.0, schemars 1.2.3, schemars_derive 1.2.3, serde-wasm-bindgen 0.6.5, sharded-slab 0.1.7, simd-adler32 0.3.10, simd_helpers 0.1.0, slab 0.4.12, sledgehammer_bindgen 0.6.0, sledgehammer_bindgen_macro 0.6.5, sledgehammer_utils 0.3.1, strsim 0.11.1, synstructure 0.14.0, tiff 0.11.3, tokio 1.54.1, tokio-macros 2.7.2, tokio-stream 0.1.19, tokio-util 0.7.20, tower 0.5.3, tower-http 0.6.11, tower-layer 0.3.3, tower-service 0.3.3, tracing 0.1.44, tracing-attributes 0.1.31, tracing-core 0.1.36, tracing-log 0.2.0, tracing-serde 0.2.0, tracing-subscriber 0.3.23, try-lock 0.2.5, unsafe-libyaml-norway 0.2.15, want 0.3.2, which 8.0.6, winnow 0.7.15, winnow 1.0.4, y4m 0.8.0, zmij 1.0.23

### Unicode-3.0 (18)

icu_collections 2.4.0, icu_locale_core 2.4.0, icu_normalizer 2.4.0, icu_normalizer_data 2.4.0, icu_properties 2.4.0, icu_properties_data 2.4.0, icu_provider 2.4.0, litemap 0.8.3, potential_utf 0.1.6, tinystr 0.8.5, writeable 0.6.4, yoke 0.8.4, yoke-derive 0.8.4, zerofrom 0.1.8, zerofrom-derive 0.1.8, zerotrie 0.2.5, zerovec 0.11.9, zerovec-derive 0.11.7

### Apache-2.0 OR MIT OR Zlib (14)

bytemuck 1.25.2, dispatch2 0.3.1, miniz_oxide 0.8.9, miniz_oxide 0.9.1, objc2-app-kit 0.3.2, objc2-core-foundation 0.3.2, objc2-core-graphics 0.3.2, objc2-exception-helper 0.1.1, objc2-web-kit 0.3.2, raw-window-handle 0.5.2, raw-window-handle 0.6.2, zune-core 0.5.3, zune-inflate 0.2.54, zune-jpeg 0.5.15

### Apache-2.0 (8)

bollard 0.21.1, bollard-stubs 1.53.1-rc.29.3.1, bytesize 2.7.0, rmcp 3.7.0, rmcp-macros 3.7.0, similar 3.2.1, sync_wrapper 1.0.2, tao 0.34.8

### Zlib (7)

const_format 0.2.37, const_format_proc_macros 0.2.37, foldhash 0.2.0, konst 0.2.20, konst_macro_rules 0.2.19, slotmap 1.1.1, zlib-rs 0.6.8

### BSD-3-Clause (5)

avif-serialize 0.8.9, exr 1.74.2, lebe 0.5.3, ravif 0.13.0, subtle 2.6.1

### MIT OR Unlicense (4)

aho-corasick 1.1.5, byteorder 1.5.0, byteorder-lite 0.1.0, memchr 2.8.3

### Apache-2.0 OR ISC OR MIT (3)

hyper-rustls 0.27.10, rustls 0.23.46, rustls-native-certs 0.8.5

### BSD-2-Clause (3)

av1-grain 0.2.5, rav1e 0.8.1, v_frame 0.3.9

### ISC (3)

libloading 0.8.9, rustls-webpki 0.103.16, untrusted 0.9.0

### Apache-2.0 OR BSD-2-Clause OR MIT (2)

zerocopy 0.8.63, zerocopy-derive 0.8.63

### Apache-2.0 OR BSD-3-Clause (2)

moxcms 0.8.1, pxfm 0.1.30

### MPL-2.0 (2)

nucleo-matcher 0.3.1, option-ext 0.2.0

These are used unmodified; their source is the crates.io package of the same name and version (MPL-2.0 file-level terms).

### (MIT OR Apache-2.0) AND Unicode-3.0 (1)

unicode-ident 1.0.27

### 0BSD OR Apache-2.0 OR MIT (1)

adler2 2.0.1

### Apache-2.0 AND ISC (1)

ring 0.17.14

### Apache-2.0 AND MIT (1)

dpi 0.1.2

### Apache-2.0 OR BSL-1.0 (1)

ryu 1.0.23

### Apache-2.0 OR CC0-1.0 (1)

imgref 1.12.3

### Apache-2.0 OR CC0-1.0 OR MIT-0 (1)

dunce 1.0.5

### Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT (1)

rustix 1.1.5
