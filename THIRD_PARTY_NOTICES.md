# Third-party notices

ED Corai's own code is under the MIT License (see `LICENSE`). Builds of ED Corai (`dist/`, the Rust
crates) also contain the following third-party material.

## Coriolis calculation engine

`upstream/coriolis` (https://github.com/EDCD/coriolis) is bundled into `dist/server.cjs` and
`dist/cli.cjs`. Its code is used under the MIT License:

```
Copyright (c) 2015 Coriolis.io, Colin McLeod

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software (Javascript, CSS, HTML, and SVG files only), and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.
```

## Elite Dangerous game data

`upstream/coriolis-data` (https://github.com/EDCD/coriolis-data) supplies ship and module data,
which is embedded in `dist/` and in the `corai-data` crate. The data is intellectual property and
copyright of Frontier Developments plc and is subject to Frontier's terms and conditions
(https://www.frontierstore.net/terms-and-conditions/). It is not covered by ED Corai's MIT License.

## npm and Cargo dependencies

Packages installed by `npm install` and `cargo build` keep their own licences, listed in their
package metadata.
