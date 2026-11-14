<script setup lang="ts">
/**
 * 动态背景：云海列车。
 *
 * 一枚全屏 fragment shader 画完整个画面，没有模型、没有贴图资源：
 *   - 天空：三段垂直渐变 + 落日圆盘与光晕；
 *   - 云海：12 层云带各按自己的速度横向漂移，越远越高越亮，越近越低越暗，
 *     层与层只靠明度区分，形成视差纵深（云顶轮廓由一张程序生成的噪声纹理采样得到）；
 *   - 桥：桥面 + 双塔 + 抛物线主缆 + 等距吊索，实心剪影；
 *   - 列车：低多边形四节车厢驶过桥面，车窗夜里透暖光，车头拖一串上升的蒸汽。
 *
 * 与 IslandScene 的取舍不同：这里刻意不引 three.js —— 一个全屏 quad 用不到场景图，
 * 裸 WebGL2 省掉整个引擎的解析与内存开销，分包体积只有几 KB 的 shader 字符串。
 *
 * 性能：像素比、目标帧率、着色器细节全部来自画质档位（`quality.ts`，用户可在
 * 设置页切换三档）；另外窗口不可见时整帧跳过、prefers-reduced-motion 只画一帧、
 * WebGL 不可用静默回落到渐变底色。
 */
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import { resolveQuality, type BgQualityProfile } from "./quality";

const props = withDefaults(
  defineProps<{
    /** 深色模式（跟随 settings.theme） */
    dark: boolean;
    /** 主题强调色 #RRGGBB，用于日晕外圈与云海雾气 */
    accent?: string;
    /** 画质档位；不传时按默认档（均衡） */
    quality?: BgQualityProfile;
  }>(),
  {
    accent: "#f0a63c",
    // 对象类默认值必须用工厂函数，否则同一份 profile 会被所有实例共享引用
    quality: () => resolveQuality(null),
  },
);

const canvasRef = ref<HTMLCanvasElement | null>(null);

/* ------------------------------------------------------------------ *
 * 调色板
 * ------------------------------------------------------------------ */

interface Palette {
  skyTop: string;
  skyMid: string;
  skyLow: string;
  sun: string;
  cloudFar: string;
  cloudMid: string;
  cloudNear: string;
  /** 桥与列车的剪影色 */
  silhouette: string;
  /** 车窗（夜间透出的暖光 / 白天的深色玻璃） */
  window: string;
  /** 1 = 夜，0 = 昼；驱动太阳亮度、车窗亮度与整体对比 */
  night: number;
}

const PALETTE_DARK: Palette = {
  skyTop: "#070a18",
  skyMid: "#3e2b4a",
  skyLow: "#6e4350",
  sun: "#ff9f5e",
  cloudFar: "#b07a78",
  cloudMid: "#6a5570",
  cloudNear: "#2b2035",
  silhouette: "#08060e",
  window: "#ffd18a",
  night: 1,
};

const PALETTE_LIGHT: Palette = {
  skyTop: "#4a95c9",
  skyMid: "#bcd9ee",
  skyLow: "#cfe4f2",
  sun: "#ffe2b0",
  cloudFar: "#fbfdff",
  cloudMid: "#b8d1e6",
  cloudNear: "#7e9cba",
  silhouette: "#36506a",
  window: "#4e6b82",
  night: 0,
};

/* ------------------------------------------------------------------ *
 * 着色器
 * ------------------------------------------------------------------ */

const VERT = `#version 300 es
void main() {
  // 全屏三角形：不需要任何顶点缓冲
  vec2 v = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
  gl_Position = vec4(v * 2.0 - 1.0, 0.0, 1.0);
}`;

const FRAG = `#version 300 es
precision highp float;

uniform vec2 uRes;
uniform float uTime;
uniform sampler2D uNoise;
uniform vec3 uSkyTop;
uniform vec3 uSkyMid;
uniform vec3 uSkyLow;
uniform vec3 uSunCol;
uniform vec3 uCloudFar;
uniform vec3 uCloudMid;
uniform vec3 uCloudNear;
uniform vec3 uSil;
uniform vec3 uWinCol;
uniform vec3 uAccent;
uniform float uNight;
// 画质档：0 流畅 / 1 均衡 / 2 高清。控制云层数与云团细节采样次数
uniform float uLevel;
// 胶片颗粒强度，低画质档置 0（省一次纹理采样）
uniform float uGrain;

out vec4 fragColor;

// 云层数上限（高清档），实际画几层由 uLevel 决定
const int MAX_LAYERS = 10;

// —— 桥的尺寸（世界坐标：画面高 = 1.0，画面中线 = 0）——
const float DECK_Y = 0.075;
const float TOWER_TOP = 0.425;
const float CABLE_SAG = 0.245;
const float SPAN = 1.30;        // 一个桥跨的长度
const float HALF_SPAN = 0.65;

// —— 摄影机跟随列车：列车钉在屏幕上，世界反向流过 ——
const float TRAIN_X = -0.05;
const float TRAIN_SPEED = 0.17;  // 世界单位 / 秒

float nz(vec2 p) { return texture(uNoise, p).r; }

/**
 * |x| < edge 时返回 1，边缘 w 宽度内平滑过渡。
 * 注意必须写成 1.0 - smoothstep(小, 大, x)：GLSL 规范里 edge0 >= edge1 是
 * 未定义行为，反写 smoothstep 在不同驱动上可能恒返回 0（曾经让星空和蒸汽整个消失）。
 */
float aaStep(float edge, float x, float w) {
  return 1.0 - smoothstep(edge - w, edge + w, x);
}

/** 云层颜色：远端偏暖（被落日照亮），近端几乎全黑 */
vec3 cloudColor(float fi) {
  return fi < 0.5
    ? mix(uCloudFar, uCloudMid, fi * 2.0)
    : mix(uCloudMid, uCloudNear, (fi - 0.5) * 2.0);
}

/**
 * 画一层云。fi 0 = 最远，1 = 最近。
 *
 * 视差是这套画面纵深的唯一来源：远云几乎不动（parallax 0.16），
 * 近云比列车本身还快（2.4），层与层之间才有速度差。
 */
vec3 drawCloud(vec3 col, vec2 p, float fi, vec2 sunPos, float t, float travel) {
  float baseY = mix(0.020, -0.440, fi);
  float amp = mix(0.090, 0.150, fi);
  // 越近的云，屏幕上的团块越大、频率越低
  float scale = mix(0.62, 0.32, fi);
  float parallax = mix(0.16, 2.40, fi);
  float drift = mix(0.010, 0.050, fi);

  // 层带：以 baseY 为中心的高斯权重，上下都衰减。
  // 上下都衰减是必须的 —— 只往下填充会让每层变成实心块，叠完就是一坨均匀的雾。
  float ty = (p.y - baseY) / amp;
  float band = exp(-ty * ty * 1.15);

  // 云团：二维噪声，同一层内部有浓有淡，才不是一条平推的色带
  vec2 c = vec2((p.x + travel * parallax + t * drift) * scale, p.y * scale * 1.35 + fi * 3.7);
  float a = nz(c);
  float b = nz(c * 1.9 + vec2(0.0, 0.5));
  float n = a * 0.68 + b * 0.32;
  // 均衡档以上再多叠一层高频，云缘更细碎；流畅档省掉这次采样
  if (uLevel > 0.5) {
    float d = nz(c * 3.6 + vec2(0.0, 0.77));
    n = a * 0.58 + b * 0.30 + d * 0.12;
  }

  // 阈值给宽一点，否则云面会浮出一层砂岩似的颗粒
  float mask = band * smoothstep(0.30, 0.70, n);
  float opa = mix(0.55, 0.92, fi);

  vec3 lc = cloudColor(fi);

  // 云团越往上越亮，下面的云体压暗，撑出体积
  float shade = clamp(0.5 - ty * 0.85, 0.0, 1.0);
  // 白天的云整体高亮，暗部得压得更狠一点，否则白底上分不出层
  lc *= 1.0 - shade * mix(0.52, 0.30, uNight);
  lc += uSunCol * shade * 0.18 * (1.0 - fi * 0.7);

  // 落日方向的漫射光晕铺在云面上
  float sg = exp(-length((p - sunPos) * vec2(0.85, 1.7)) * 2.6);
  lc += uSunCol * sg * 0.26 * (1.0 - fi * 0.9);

  return mix(col, lc, mask * opa);
}

/**
 * 悬索桥剪影遮罩。q 是「世界坐标」——横向已经叠加了列车行驶距离，
 * 所以桥塔、吊索、栏杆都会随列车前进向后退。
 */
float bridgeMask(vec2 q) {
  float m = 0.0;

  // 桥面
  m = max(m, aaStep(0.0075, abs(q.y - DECK_Y), 0.0016));
  // 桥面下方的桁架暗带
  m = max(m, aaStep(0.016, abs(q.y - (DECK_Y - 0.017)), 0.004) * 0.55);

  // 跨内局部坐标：[-HALF_SPAN, HALF_SPAN)，桥塔落在两端
  float local = mod(q.x + HALF_SPAN, SPAN) - HALF_SPAN;
  float u = local / HALF_SPAN;

  // 双塔（每个跨两端各一座）：从塔顶一直伸到画面外，下半截被近景云遮住
  float dTower = abs(abs(local) - HALF_SPAN);
  m = max(m, aaStep(0.0095, dTower, 0.0016) * step(q.y, TOWER_TOP));
  m = max(m, aaStep(0.042, dTower, 0.0016) * aaStep(0.013, abs(q.y - TOWER_TOP), 0.0016));

  // 主缆：塔顶最高、跨中下垂的抛物线
  float cableY = CABLE_SAG + (TOWER_TOP - CABLE_SAG) * u * u;
  m = max(m, aaStep(0.0035, abs(q.y - cableY), 0.0016) * step(DECK_Y - 0.02, q.y));

  // 吊索：等距竖线，只在主缆与桥面之间
  float stepX = 0.058;
  float dx = abs(mod(q.x + stepX * 0.5, stepX) - stepX * 0.5);
  float hanger = aaStep(0.0015, dx, 0.0008) * step(q.y, cableY) * step(DECK_Y, q.y);
  m = max(m, hanger * 0.9);

  // 栏杆立柱：桥面上唯一能读出「在往前走」的远景细节
  float railX = abs(mod(q.x, 0.048) - 0.024);
  float rail = aaStep(0.0011, railX, 0.0007) * step(DECK_Y + 0.006, q.y) * step(q.y, DECK_Y + 0.020);
  m = max(m, rail * 0.85);

  return m;
}

/**
 * 列车剪影 + 车窗。横向位置固定（摄影机跟着它走），只保留极轻微的车厢起伏。
 * 返回 (车身遮罩, 车窗遮罩)。车头朝右，四节车厢沿 -x 方向排开。
 */
vec2 trainMask(vec2 p, float t) {
  const float CAR_W = 0.104;
  const float CAR_GAP = 0.011;
  const float CAR_H = 0.058;

  // 桥面弹性带来的轻微起伏，幅度必须极小，否则像在颠簸
  float bob = sin(t * 1.7) * 0.0014;
  float bottom = DECK_Y + 0.010 + bob;
  float top = bottom + CAR_H;

  float body = 0.0;
  float win = 0.0;

  for (int i = 0; i < 4; i++) {
    float cx = TRAIN_X - float(i) * (CAR_W + CAR_GAP);
    float halfW = CAR_W * 0.5;

    body = max(body, aaStep(halfW, abs(p.x - cx), 0.0018) * step(bottom, p.y) * step(p.y, top));

    // 车头：右端接一个向下收的斜鼻
    if (i == 0) {
      float nx = p.x - (cx + halfW);
      float ny = clamp((p.y - bottom) / CAR_H, 0.0, 1.0);
      float noseW = 0.030 * (1.0 - ny);
      body = max(body, step(0.0, nx) * step(nx, noseW) * step(bottom, p.y) * step(p.y, top));
      // 烟囱：车头顶上一小截
      body = max(body, aaStep(0.010, abs(p.x - (cx + 0.014)), 0.0012) * step(top, p.y) * step(p.y, top + 0.012));
    }

    // 车钩：车厢之间的连接
    body = max(body, aaStep(0.0055, abs(p.x - (cx - halfW - CAR_GAP * 0.5)), 0.001)
      * step(bottom + 0.010, p.y) * step(p.y, bottom + 0.026));

    // 三扇车窗
    for (int k = 0; k < 3; k++) {
      float wx = cx + (float(k) - 1.0) * 0.030;
      float wy = bottom + CAR_H * 0.56;
      win = max(win, aaStep(0.0088, abs(p.x - wx), 0.0012) * aaStep(0.0062, abs(p.y - wy), 0.0012));
    }
  }

  return vec2(body, win);
}

/** 车头升起的蒸汽：几团随时间膨胀、上升、向后飘散 */
float smokeMask(vec2 p, float t) {
  float bottom = DECK_Y + 0.010;
  float chimneyX = TRAIN_X + 0.016;
  float chimneyY = bottom + 0.058;

  float m = 0.0;
  for (int i = 0; i < 7; i++) {
    float fi = float(i);
    float age = fract(t * 0.22 + fi * 0.143);
    // 一边上升一边被风吹向车尾，半径随时间膨胀
    vec2 c = vec2(chimneyX - age * 0.16, chimneyY + 0.010 + age * 0.16);
    float r = 0.012 + age * 0.030;
    float d = length((p - c) * vec2(1.0, 0.85)) / r;
    m = max(m, (1.0 - smoothstep(0.18, 1.0, d)) * (1.0 - age));
  }
  return m;
}

void main() {
  vec2 uv = gl_FragCoord.xy / uRes;
  float aspect = uRes.x / uRes.y;
  // p：屏幕空间世界坐标，纵向固定 1 单位高，横向按宽高比扩展（不会拉伸变形）
  vec2 p = (uv - 0.5) * vec2(aspect, 1.0);
  float t = uTime;

  // 列车已行驶的距离：桥、云、栏杆都按这个量反向流动
  float travel = t * TRAIN_SPEED;
  vec2 q = vec2(p.x + travel, p.y);

  // —— 天空 ——
  float h = uv.y;
  vec3 col = mix(uSkyLow, uSkyMid, smoothstep(0.0, 0.60, h));
  col = mix(col, uSkyTop, smoothstep(0.52, 1.04, h));

  // —— 落日：钉在天空里，不随列车移动 ——
  vec2 sunPos = vec2(0.34, 0.20);
  float sd = length((p - sunPos) * vec2(1.0, 1.34));
  // 白天不需要一枚白圆盘，只留一团柔光
  col += uSunCol * (1.0 - smoothstep(0.048, 0.062, sd)) * mix(0.32, 0.80, uNight);
  col += uSunCol * exp(-sd * 3.4) * 0.42;
  col += uAccent * exp(-sd * 1.4) * 0.13;

  // 地平线暖带：落日的颜色要横向铺满，只靠日轮周围一圈光晕会很假
  float band = exp(-pow((h - 0.56) * 3.1, 2.0));
  col += uSunCol * band * 0.11;

  // —— 远山影：视差极小，几乎不动 ——
  float ridge = nz(vec2((p.x + travel * 0.05) * 0.55, 0.13));
  float ridgeTop = 0.098 + (ridge - 0.5) * 0.075;
  col = mix(col, mix(uSkyLow, uSil, 0.42), (1.0 - smoothstep(ridgeTop - 0.004, ridgeTop + 0.004, p.y)) * 0.42);

  // —— 云层数量按画质档：流畅 6 层、均衡 8 层、高清 10 层 ——
  // 循环上界必须是编译期常量，所以统一写到 MAX_LAYERS 再用 break 截断
  int layers = int(mix(6.0, 10.0, uLevel * 0.5));
  float layerSpan = float(max(layers - 1, 1));
  // 前 40% 的层在桥后面，其余在桥前面（近景云遮住塔基）
  int farLayers = max(layers * 2 / 5, 1);

  // —— 远景云（在桥之后画，桥会盖在它们上面）——
  for (int i = 0; i < MAX_LAYERS; i++) {
    if (i >= farLayers) break;
    col = drawCloud(col, p, float(i) / layerSpan, sunPos, t, travel);
  }

  // —— 桥与列车 ——
  // 大气透视：越靠近落日，剪影越不该是死黑
  float nearSun = exp(-length((p - sunPos) * vec2(0.72, 1.25)) * 1.9);
  vec3 silCol = mix(uSil, uSunCol * 0.55 + uSil, nearSun * 0.55);

  float bm = bridgeMask(q);
  col = mix(col, silCol, bm);

  vec2 tr = trainMask(p, t);
  col = mix(col, silCol, tr.x);
  // 车窗：夜间透暖光，白天是深色玻璃
  col = mix(col, uWinCol, tr.y * mix(0.85, 1.0, uNight));

  // 车灯：夜里车头前方一小团暖光
  vec2 lampPos = vec2(TRAIN_X + 0.058, DECK_Y + 0.040);
  float lampD = length(p - lampPos);
  col += uWinCol * exp(-lampD * 26.0) * 0.55 * uNight;

  // 蒸汽：常年飘在落日前面，必须是偏冷的白，跟暖橙的日轮拉开色相差
  float sm = smokeMask(p, t);
  vec3 smokeCol = mix(vec3(0.90, 0.92, 0.96), uSunCol, 0.22) * (0.92 + 0.10 * uNight);
  col = mix(col, smokeCol, sm);

  // —— 近景云（遮住桥塔下半截，做出纵深）——
  for (int i = 0; i < MAX_LAYERS; i++) {
    if (i < farLayers) continue;
    if (i >= layers) break;
    col = drawCloud(col, p, float(i) / layerSpan, sunPos, t, travel);
  }

  // 桥面一带的雾气：让桥像从云海中浮起
  float mist = exp(-abs(p.y - DECK_Y + 0.03) * 5.0) * 0.16;
  col = mix(col, mix(uSkyMid, uAccent, 0.25), mist);

  // —— 后处理：暗角 + 细颗粒 ——
  float vig = 1.0 - 0.32 * pow(length((uv - 0.5) * vec2(1.06, 1.16)) * 1.42, 2.2);
  col *= clamp(vig, 0.0, 1.0);

  if (uGrain > 0.0) {
    float grain = nz(gl_FragCoord.xy / uRes * 3.7 + fract(t) * 11.0) - 0.5;
    col += grain * uGrain;
  }

  fragColor = vec4(clamp(col, 0.0, 1.0), 1.0);
}`;

/* ------------------------------------------------------------------ *
 * 噪声纹理（程序生成，可无缝平铺）
 * ------------------------------------------------------------------ */

function hash2(x: number, y: number, seed: number): number {
  let h = Math.imul(x, 374761393) ^ Math.imul(y, 668265263) ^ Math.imul(seed, 1013904223);
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  h ^= h >>> 16;
  return (h >>> 0) / 4294967295;
}

/** 周期化 value noise：period 是格点数，保证 tile 时左右/上下无缝 */
function pnoise(x: number, y: number, period: number, seed: number): number {
  const ix = Math.floor(x);
  const iy = Math.floor(y);
  const fx = x - ix;
  const fy = y - iy;
  const ux = fx * fx * (3 - 2 * fx);
  const uy = fy * fy * (3 - 2 * fy);
  const wrap = (v: number) => ((v % period) + period) % period;
  const x0 = wrap(ix);
  const x1 = wrap(ix + 1);
  const y0 = wrap(iy);
  const y1 = wrap(iy + 1);
  const a = hash2(x0, y0, seed);
  const b = hash2(x1, y0, seed);
  const c = hash2(x0, y1, seed);
  const d = hash2(x1, y1, seed);
  return (a * (1 - ux) + b * ux) * (1 - uy) + (c * (1 - ux) + d * ux) * uy;
}

function fbm(u: number, v: number, seed: number, basePeriod: number, octaves: number): number {
  let sum = 0;
  let amp = 0.5;
  let total = 0;
  let period = basePeriod;
  for (let o = 0; o < octaves; o++) {
    sum += pnoise(u * period, v * period, period, seed + o * 17) * amp;
    total += amp;
    amp *= 0.5;
    period *= 2;
  }
  return sum / total;
}

/** 生成 256×256 噪声纹理：R = 云形主通道，G = 细节通道，B = 颗粒 */
function buildNoiseTexture(gl: WebGL2RenderingContext): WebGLTexture | null {
  const size = 256;
  const data = new Uint8Array(size * size * 4);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const u = x / size;
      const v = y / size;
      const r = fbm(u, v, 1, 6, 4);
      const g = fbm(u, v, 91, 12, 3);
      const b = hash2(x, y, 7) * 255;
      const i = (y * size + x) * 4;
      data[i] = Math.round(r * 255);
      data[i + 1] = Math.round(g * 255);
      data[i + 2] = b;
      data[i + 3] = 255;
    }
  }

  const tex = gl.createTexture();
  if (!tex) return null;
  gl.bindTexture(gl.TEXTURE_2D, tex);
  gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, size, size, 0, gl.RGBA, gl.UNSIGNED_BYTE, data);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.REPEAT);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.REPEAT);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
  return tex;
}

/* ------------------------------------------------------------------ *
 * 渲染生命周期
 * ------------------------------------------------------------------ */

/** 当前画质档位；档位变化由父组件的 :key 触发重建，所以这里取一次就够 */
const q = props.quality;

let gl: WebGL2RenderingContext | null = null;
let prog: WebGLProgram | null = null;
let noiseTex: WebGLTexture | null = null;
let raf = 0;
let observer: ResizeObserver | null = null;
let startedAt = 0;
let lastFrameAt = 0;
let active = false;

const uniforms: Record<string, WebGLUniformLocation | null> = {};

function hexToRgb(hex: string): [number, number, number] {
  const h = hex.replace("#", "");
  const n = parseInt(h.length === 3 ? h.split("").map((c) => c + c).join("") : h, 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255].map((v) => v / 255) as [number, number, number];
}

function compile(glc: WebGL2RenderingContext, type: number, src: string): WebGLShader | null {
  const sh = glc.createShader(type);
  if (!sh) return null;
  glc.shaderSource(sh, src);
  glc.compileShader(sh);
  if (!glc.getShaderParameter(sh, glc.COMPILE_STATUS)) {
    console.warn("[CloudTrainScene] shader 编译失败", glc.getShaderInfoLog(sh));
    glc.deleteShader(sh);
    return null;
  }
  return sh;
}

function applyPalette() {
  if (!gl || !prog) return;
  // uniform 只能写到「当前 program」上，没有这行所有颜色都会停在默认值 0（整屏全黑）
  gl.useProgram(prog);
  const pal = props.dark ? PALETTE_DARK : PALETTE_LIGHT;
  const set3 = (name: string, hex: string) => {
    const loc = uniforms[name];
    if (loc) gl!.uniform3f(loc, ...hexToRgb(hex));
  };
  set3("uSkyTop", pal.skyTop);
  set3("uSkyMid", pal.skyMid);
  set3("uSkyLow", pal.skyLow);
  set3("uSunCol", pal.sun);
  set3("uCloudFar", pal.cloudFar);
  set3("uCloudMid", pal.cloudMid);
  set3("uCloudNear", pal.cloudNear);
  set3("uSil", pal.silhouette);
  set3("uWinCol", pal.window);
  set3("uAccent", props.accent || "#f0a63c");
  if (uniforms.uNight) gl.uniform1f(uniforms.uNight, pal.night);
}

function resize() {
  const c = canvasRef.value;
  if (!gl || !c) return;
  const dpr = q.pixelRatio(window.devicePixelRatio || 1);
  const w = Math.max(1, Math.round(c.clientWidth * dpr));
  const h = Math.max(1, Math.round(c.clientHeight * dpr));
  if (c.width === w && c.height === h) return;
  c.width = w;
  c.height = h;
  gl.viewport(0, 0, w, h);
  // 用当前时间重绘：拿固定帧号画会把画面弹回起点，缩放窗口时会看到一次跳变
  if (active) draw((performance.now() - startedAt) / 1000);
  else renderOnce();
}

function draw(t: number) {
  const c = canvasRef.value;
  if (!gl || !prog || !c) return;
  gl.useProgram(prog);
  if (uniforms.uRes) gl.uniform2f(uniforms.uRes, c.width, c.height);
  if (uniforms.uTime) gl.uniform1f(uniforms.uTime, t);
  if (uniforms.uLevel) gl.uniform1f(uniforms.uLevel, q.level);
  // 颗粒只是氛围，低画质档直接关掉
  if (uniforms.uGrain) gl.uniform1f(uniforms.uGrain, q.level === 0 ? 0 : 0.016);
  gl.activeTexture(gl.TEXTURE0);
  gl.bindTexture(gl.TEXTURE_2D, noiseTex);
  if (uniforms.uNoise) gl.uniform1i(uniforms.uNoise, 0);
  gl.drawArrays(gl.TRIANGLES, 0, 3);
}

function renderOnce() {
  draw(12);
}

function loop(now: number) {
  raf = requestAnimationFrame(loop);
  if (document.hidden) return;
  // 留 1.5ms 余量：rAF 间隔本身就在 16.6~16.7ms 之间抖，阈值取 1000/60 整
  // 会让 60 档隔帧才画一次（实际只有 30fps）
  if (now - lastFrameAt < 1000 / q.fps - 1.5) return;
  lastFrameAt = now;
  draw((now - startedAt) / 1000);
}

function init() {
  const c = canvasRef.value;
  if (!c) return;
  // antialias / powerPreference 只在建上下文时生效，所以切档位靠父组件重建组件
  const ctx = c.getContext("webgl2", {
    // 全屏 shader 没有几何边缘，MSAA 帮不上忙；高清档才付这份显存
    antialias: q.level === 2,
    alpha: false,
    depth: false,
    stencil: false,
    powerPreference: q.level === 0 ? "low-power" : "default",
  });
  if (!ctx) {
    // 没有 WebGL2：静默隐藏画布，底层的渐变背景自动顶上
    c.style.display = "none";
    return;
  }
  gl = ctx;

  const vs = compile(gl, gl.VERTEX_SHADER, VERT);
  const fs = compile(gl, gl.FRAGMENT_SHADER, FRAG);
  if (!vs || !fs) {
    c.style.display = "none";
    return;
  }
  prog = gl.createProgram();
  if (!prog) return;
  gl.attachShader(prog, vs);
  gl.attachShader(prog, fs);
  gl.linkProgram(prog);
  gl.deleteShader(vs);
  gl.deleteShader(fs);
  if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) {
    console.warn("[CloudTrainScene] program 链接失败", gl.getProgramInfoLog(prog));
    c.style.display = "none";
    return;
  }

  noiseTex = buildNoiseTexture(gl);

  // 全屏三角形不需要顶点数据，但 WebGL2 要求绑定一个 VAO 才能用 gl_VertexID
  const vao = gl.createVertexArray();
  gl.bindVertexArray(vao);

  for (const name of [
    "uRes", "uTime", "uNoise", "uSkyTop", "uSkyMid", "uSkyLow", "uSunCol",
    "uCloudFar", "uCloudMid", "uCloudNear", "uSil", "uWinCol", "uAccent", "uNight",
    "uLevel", "uGrain",
  ]) {
    uniforms[name] = gl.getUniformLocation(prog, name);
  }

  applyPalette();
  resize();

  const reduced = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
  if (reduced) {
    // 减少动态效果：只给一帧静止画面；尺寸仍要跟着变，否则窗口一缩放就拉伸
    if (typeof ResizeObserver !== "undefined") {
      observer = new ResizeObserver(() => resize());
      observer.observe(c);
    }
    window.addEventListener("resize", resize);
    renderOnce();
    return;
  }

  active = true;
  startedAt = performance.now();
  lastFrameAt = 0;
  // ResizeObserver 盯画布自身尺寸：窗口最大化、侧栏折叠、DPI 切换这些场景
  // 不一定触发 window.resize，只靠它会出现画布尺寸没跟上、整幅画面被拉伸变形
  if (typeof ResizeObserver !== "undefined") {
    observer = new ResizeObserver(() => resize());
    observer.observe(c);
  }
  window.addEventListener("resize", resize);
  raf = requestAnimationFrame(loop);
}

function destroy() {
  cancelAnimationFrame(raf);
  raf = 0;
  active = false;
  observer?.disconnect();
  observer = null;
  window.removeEventListener("resize", resize);
  if (gl) {
    if (noiseTex) gl.deleteTexture(noiseTex);
    if (prog) gl.deleteProgram(prog);
    // 主动丢上下文：反复切换背景时避免 WebGL context 堆积
    gl.getExtension("WEBGL_lose_context")?.loseContext();
  }
  gl = null;
  prog = null;
  noiseTex = null;
}

onMounted(init);
onBeforeUnmount(destroy);

watch(
  () => props.dark,
  () => {
    applyPalette();
    if (!active) renderOnce();
  },
);

watch(
  () => props.accent,
  () => applyPalette(),
);
</script>

<template>
  <canvas ref="canvasRef" class="cloud-train-scene" aria-hidden="true"></canvas>
</template>

<style scoped>
.cloud-train-scene {
  position: absolute;
  inset: 0;
  display: block;
  width: 100%;
  height: 100%;
  pointer-events: none;
}
</style>
