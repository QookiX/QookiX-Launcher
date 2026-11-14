# 第三方组件声明 (Third-Party Notices)

本文件列出了 QookiX Launcher 所使用或所集成的第三方开源软件及其许可证信息，以满足相应开源许可协议的署名（attribution）要求。

## Terracotta（陶瓦联机）

- **项目**：Terracotta（陶瓦联机）
- **作者 / 版权**：Copyright © burningtnt
- **仓库**：https://github.com/burningtnt/Terracotta
- **许可证**：GNU Affero General Public License v3.0 or later（**AGPL-3.0-or-later**）
- **完整许可证文本**：https://github.com/burningtnt/Terracotta/blob/master/LICENSE
- **集成方式**：独立运行的外部进程

**使用说明**：QookiX Launcher 的「联机房间」功能使用 Terracotta 提供 NAT 穿透联机能力。本启动器仅负责下载 Terracotta 的**未修改官方二进制**并作为独立进程启动，再通过其**本地 HTTP 接口**（进程间通信）与之交互，**不将 Terracotta 静态或动态链接进本启动器**。

根据 Terracotta 许可证所附的 **AGPL 例外条款**，上述「打包未修改二进制 / 通过进程间通信接口交互」的使用方式不会导致本启动器被 AGPL 协议涵盖。作为该例外条款的条件之一，本启动器已在程序界面的「联机房间」页面明显处标识了 Terracotta 的版权信息。

Terracotta 与本启动器是两个相互独立、各自以自身许可证发布的开源项目，二者之间不存在任何隶属或背书关系。

---

## cubiomes

- **项目**：cubiomes
- **作者 / 版权**：Copyright © 2020 Cubitect
- **仓库**：https://github.com/Cubitect/cubiomes
- **许可证**：MIT License
- **完整许可证文本**：https://github.com/Cubitect/cubiomes/blob/master/LICENSE
- **集成方式**：Vendored 源码快照，静态链接进本启动器

**使用说明**：「工具箱 → 种子地图」的生物群系、结构与出生点计算由 cubiomes 提供。本启动器在 `src-tauri/vendor/cubiomes/` 保存其**上游未修改的源码快照**（含其 LICENSE 与 README），由 `src-tauri/build.rs` 编译，并通过自写的 C 胶水层（`src-tauri/src/cubiomes_bridge.c`）与 Rust 封装（`src-tauri/src/cubiomes.rs`）调用；本启动器未修改 cubiomes 本身。

cubiomes 采用 MIT 许可证，与本项目采用的 GNU GPL v3 兼容。cubiomes 与本启动器是两个相互独立、各自以自身许可证发布的开源项目，二者之间不存在任何隶属或背书关系。

### MIT License

```
MIT License

Copyright (c) 2020 Cubitect

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

---

## Feather Icons

- **项目**：Feather Icons
- **作者 / 版权**：Copyright (c) 2013-2017 Cole Bemis
- **来源**：https://feathericons.com/
- **许可证**：MIT License
- **集成方式**：本启动器界面图标改编自 Feather Icons

### MIT License

```
MIT License

Copyright (c) 2013-2017 Cole Bemis

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

---

## Lucide Icons

- **项目**：Lucide
- **作者 / 版权**：Copyright (c) 2020 Lucide Contributors
- **来源**：https://lucide.dev/
- **许可证**：ISC License
- **完整许可证文本**：https://github.com/lucide-icons/lucide/blob/main/LICENSE
- **集成方式**：本启动器界面图标改编自 Lucide

### ISC License

```
ISC License

Copyright (c) for portions of project Lucide are held by Cole Bemis 2013-2017 as part of project Feather.
All other copyright (c) for project Lucide are held by Lucide Contributors 2020.

Permission to use, copy, modify, and/or distribute this software for any
purpose with or without fee is hereby granted, provided that the above
copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
```

---

## GitHub Octicons

- **项目**：GitHub Octicons（mark-github）
- **作者 / 版权**：Copyright © GitHub, Inc.
- **来源**：https://github.com/primer/octicons
- **许可证**：MIT License（完整文本见上方 Feather Icons 条目中的 MIT License 全文）
- **完整许可证文本**：https://github.com/primer/octicons/blob/main/LICENSE
- **集成方式**：本启动器界面中的 GitHub 标识图形改编自 Octicons

---

## MohuaFrp（frp 内网穿透服务）

- **项目**：MohuaFrp
- **服务地址**：https://frp.mhjz1.cn
- **集成方式**：通过 API 对接，使用 frpc 客户端进行端口映射
- **服务条款**：https://frp.mhjz1.cn/terms
- **许可证**：服务条款见官网（闭源服务）

**使用说明**：
本启动器的「内网穿透」功能通过 MohuaFrp 服务提供端口映射能力。用户需自行注册账号并创建隧道，本启动器仅负责：
1. 设备授权登录（OAuth2 设备码流程）
2. 隧道列表展示与启停
3. 本地隧道配置（自定义端口、协议等）
4. frpc 子进程管理

**重要提示**：
- MohuaFrp 是第三方闭源服务，与本启动器无任何隶属或背书关系
- 用户需遵守 MohuaFrp 的服务条款
- 本启动器不分发 frpc 二进制文件，用户需自行从官方源下载
- frpc 采用 Apache-2.0 许可证，与本项目 GPL-3.0 兼容

**免责声明**：
本启动器不对用户通过内网穿透功能进行的任何行为承担责任。用户需自行遵守相关法律法规。

MohuaFrp 与本启动器是两个相互独立、各自以自身许可证发布的第三方服务，二者之间不存在任何隶属或背书关系。

---

## frpc（frp 客户端）

- **项目**：frp
- **作者 / 版权**：Copyright (c) 2016-2024 frp Authors
- **仓库**：https://github.com/fatedier/frp
- **许可证**：Apache License 2.0（**Apache-2.0**）
- **完整许可证文本**：https://github.com/fatedier/frp/blob/master/LICENSE
- **集成方式**：外部二进制文件，用户自行下载

**使用说明**：
frpc 是 frp 项目的客户端组件，用于实现内网穿透功能。本启动器不静态或动态链接 frpc，而是通过子进程方式调用 frpc 可执行文件。用户需自行从 frp 官方发行包中提取 frpc 可执行文件并放置到指定路径（`<数据目录>/frp/frpc.exe`）。

frpc 采用 Apache-2.0 许可证，与本项目采用的 GNU GPL v3 兼容。frpc 与本启动器是两个相互独立、各自以自身许可证发布的开源项目，二者之间不存在任何隶属或背书关系。

### Apache License 2.0

```
Apache License
                           Version 2.0, January 2004
                        http://www.apache.org/licenses/

   TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION

   1. Definitions.

      "License" shall mean the terms and conditions for use, reproduction,
      and distribution as defined by Sections 1 through 9 of this document.

      "Licensor" shall mean the copyright owner or entity authorized by
      the copyright owner that is granting the License.

      "Legal Entity" shall mean the union of the acting entity and all
      other entities that control, are controlled by, or are under common
      control with that entity. For the purposes of this definition,
      "control" means (i) the power, direct or indirect, to cause the
      direction or management of such entity, whether by contract or
      otherwise, (ii) ownership of more than fifty percent (50%) of the
      outstanding shares or beneficial ownership of such entity, or
      (iii) the right to appoint a majority of the directors or other
      officers of such entity.

      "You" (or "Your") shall mean an individual or Legal Entity
      exercising permissions granted by this License.

      "Source" form shall mean the preferred form for making modifications,
      including but not limited to software source code, documentation
      source, and configuration files.

      "Object" form shall mean any form resulting from mechanical
      transformation or translation of a Source form, including but
      not limited to compiled object code, generated documentation,
      and conversions to other media types.

      "Work" shall mean the work of authorship, whether in Source or
      Object form, made available under the License, as indicated by a
      copyright notice that is included in or attached to the work.

      "Derivative Works" shall mean, whether in whole or in part, any
      work, whether in Source or Object form, that is based on (or
      derived from) the Work and for which the editorial revisions,
      annotations, elaborations, or other modifications represent, as
      a whole, an original work of authorship. For the purposes of this
      License, Derivative Works shall not include works that remain
      separable from, or merely link (or bind by name) to the interfaces
      of, the Work and Derivative Works thereof.

      "Contribution" shall mean any work of authorship, including
      the original version of the Work and any modifications or additions
      to that Work or Derivative Works thereof, that is intentionally
      submitted to Licensor for inclusion in the Work by the copyright
      owner or by an individual or Legal Entity authorized to submit
      on behalf of the copyright owner. For the purposes of this
      definition, "submitted" means any form of electronic, verbal, or
      written communication sent to the Licensor or its representatives,
      including but not limited to communication on electronic mailing
      lists, source code control systems, and issue tracking systems
      that are managed by, or on behalf of, the Licensor for the
      purpose of discussing and improving the Work, but excluding
      communication that is conspicuously marked or otherwise designated
      in writing by the copyright owner as "Not a Contribution."

      "Contributor" shall mean Licensor and any individual or Legal
      Entity on behalf of whom a Contribution has been received by
      Licensor and subsequently incorporated within the Work.

   2. Grant of Copyright License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      copyright license to reproduce, prepare Derivative Works of,
      publicly display, publicly perform, sublicense, and distribute
      the Work and such Derivative Works in Source or Object form.

   3. Grant of Patent License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      patent license to make, have made, use, offer to sell, sell,
      import, and otherwise transfer the Work, where such license
      applies only to those patent claims licensable by such Contributor
      that are necessarily infringed by their Contribution(s) alone or
      in combination with the Work. If You institute patent litigation
      against any entity (including a cross-claim or counterclaim in
      a lawsuit) alleging that the Work or a Contribution incorporated
      within the Work constitutes direct or contributory patent
      infringement, then any patent licenses granted to You under this
      License for that Work shall terminate as of the date such
      litigation is filed.

   4. Redistribution. You may reproduce and distribute copies of the
      Work or Derivative Works thereof in any medium, with or without
      modifications, and in Source or Object form, provided that You
      meet the following conditions:

      (a) You must give any other recipients of the Work or
          Derivative Works a copy of this License; and

      (b) You must cause any modified files to carry prominent notices
          stating that You changed the files; and

      (c) You must retain, in the Source form of any Derivative Works
          that You distribute, all copyright, patent, trademark, and
          attribution notices from the Source form of the Work,
          excluding those notices that do not pertain to any part
          of the Derivative Works; and

      (d) If the Work includes a "NOTICE" text file as part of its
          distribution, then any Derivative Works that You distribute
          must include a readable copy of the attribution notices
          contained within such NOTICE file in at least one of the
          following places: within a NOTICE text file distributed as
          part of the Derivative Works; within the Source form or
          documentation for the Derivative Works, if provided; or
          within a display generated by the Derivative Works. The
          contents of the NOTICE file are for informational purposes
          only and do not modify the License. You may add Your own
          attribution notices within Derivative Works that You
          distribute, alongside or as an addendum to the NOTICE text
          from the Work, provided that such additional attribution
          notices cannot be construed as modifying the License.

   5. You may submit Your own contributions for inclusion in the Work
      by the terms of this License. You understand and agree that
      You are not required to grant any license to Your contributions
      under this License.

   6. THIS WORK IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
      EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES
      OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
      NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT
      HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY,
      WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
      FROM, OUT OF OR IN CONNECTION WITH THE WORK OR THE USE OR OTHER
      DEALINGS IN THE WORK.

   7. Limitation of Liability. In no event and under no legal theory,
      whether in tort (including negligence), contract, or otherwise,
      unless required by applicable law or agreed to in writing, shall
      any Contributor be liable to You for damages, including any
      direct, indirect, special, incidental, or consequential damages
      of any character arising as a result of this License or out of
      the use or inability to use the Work (including but not limited
      to damages for loss of goodwill, work stoppage, computer failure
      or malfunction, or any and all other commercial damages or
      losses), even if such Contributor has been advised of the
      possibility of such damages.

   8. Accepting Warranty or Additional Liability. While redistributing
      the Work or Derivative Works thereof, You may choose to offer,
      and charge a fee for, acceptance of support, warranty, indemnity,
      or other liability obligations and/or rights consistent with this
      License. However, in accepting such obligations, You may act only
      on Your own behalf and on Your sole responsibility, not on behalf
      of any other Contributor, and only if You agree to indemnify,
      defend, and hold harmless each Contributor for any liability
      incurred by, or claims asserted against, such Contributor by
      reason of your accepting any such warranty or additional liability.

   END OF TERMS AND CONDITIONS
```

---

## Axolotl Launcher

- **项目**：Axolotl Launcher
- **作者 / 版权**：Copyright © Axolotl-Launcher Contributors
- **仓库**：https://github.com/Axolotl-Launcher/Axolotl
- **许可证**：GNU General Public License v3.0（**GPL-3.0**）
- **完整许可证文本**：https://github.com/Axolotl-Launcher/Axolotl/blob/main/LICENSE

**使用说明**：本启动器以下模块的实现参考或衍生自 Axolotl Launcher（GPL-3.0），依据 GPL-3.0 第 5 条（「组合作品」/「衍生作品」）将其源码纳入本启动器并以相同许可证（GPL-3.0）分发：

1. **WebView2 现代安装器**（`src-tauri/installer-ui/`、`src-tauri/nsis/installer.nsi`）：独立的 WebView2 子 crate 与 NSIS 深色主题逻辑，参考自 Axolotl 的 `installer-ui` 模块与定制安装脚本。
2. **中文词库解析器**（`src-tauri/src/mcmod.rs`）：`@` 前后缀切片、分隔符与星号还原等解析逻辑，参考自 Axolotl 的 `packages/app-lib/src/api/content_search.rs` 中的 Rust 实现。
3. **投影工坊**（`src-tauri/src/commands/schematic.rs`、前端 `schematicMeshWorker.ts`）：分块解析、限制常量、数据结构体定义，以及 `appendFallbackCube` 占位方块绘制逻辑（含字符哈希颜色计算公式），参考自 Axolotl 的对应模块。

Axolotl Launcher 与本启动器是两个相互独立、各自以 GPL-3.0 许可证发布的开源项目，二者之间不存在任何隶属或背书关系。感谢 Axolotl Launcher 项目及其贡献者的开源工作。

---

## PCL2（Plain Craft Launcher 2）

- **项目**：PCL2（Plain Craft Launcher 2）
- **作者 / 版权**：Copyright © 龙腾猫跃（LTCat）
- **仓库**：https://github.com/LTCat/PCL2
- **许可证**：GNU General Public License v3.0（**GPL-3.0**）
- **完整许可证文本**：https://github.com/LTCat/PCL2/blob/main/LICENSE

**使用说明**：本启动器的 `WikiEntries.txt`（中文模组百科词条词库）的数据源自 PCL2 项目收录的 MCMod 百科中文词条索引。该数据由 PCL2 以 GPL-3.0 许可证分发，本启动器依据 GPL-3.0 第 5 条将其纳入并以相同许可证分发。

同时感谢 [MCMod 百科](https://www.mcmod.cn) 提供的中文 Minecraft 模组百科内容，本词库仅为其词条名称的结构化索引，不包含百科正文内容。

PCL2 与本启动器是两个相互独立、各自以 GPL-3.0 许可证发布的开源项目，二者之间不存在任何隶属或背书关系。

---

## 前端（npm）依赖

本启动器前端构建产物（Vue 3、naive-ui、skinview3d、marked、DOMPurify、driver.js 等）所使用的第三方库均为宽松许可证（MIT / Apache-2.0 / ISC / BSD 等），其许可证文本随各自包分发于 `node_modules/<包名>/LICENSE*`，完整清单以 `package.json` 中的依赖列表为准。

---

> 如对第三方组件的许可合规有任何疑问，欢迎在本项目仓库提交 Issue。
