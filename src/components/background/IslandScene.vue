<script setup lang="ts">
/**
 * 动态背景：一座低多边形海上小岛。
 *
 * 渲染链：three.js（WebGL2）→ 全屏 canvas，作为 `.app-bg` 的子层（z-index: var(--z-bg)），
 * 在内容之下、在 ::after 暗化遮罩之下，因此不需要改任何页面布局。
 *
 * 画风目标是「动画背景板」而不是写实渲染，手法上刻意避开物理正确的连续渐变：
 *   - 天空：三段分色（奶油地平线 / 青蓝天 / 钴蓝天顶），太阳是硬核 + 双圈光晕；
 *     夜间换成冷白的月亮，另加一条被噪声打散的银河；
 *   - 海面：噪声高度场 → 等值线画出的波纹笔触、贴着岛的浅滩色环、
 *     岛的倒影、太阳正下方的碎金光路、岸线泡沫；
 *   - 云：不透明积云，明暗按三次色阶硬切（赛璐璐），逆光时靠边缘透光提亮；
 *   - 收尾统一做一点点色阶量化 + 冷暖分离，压掉「CG 塑料感」。
 *
 * 性能约束（启动器是常驻后台的应用，不能一直吃满 GPU）：
 *   - 像素比、帧率、水面细分由画质档位决定（`quality.ts`，设置页可切三档）；
 *   - 窗口不可见时整帧跳过；
 *   - 系统开启「减少动态效果」时只渲染一帧静态画面；
 *   - WebGL 创建失败时静默隐藏 canvas，底层渐变背景自动顶上。
 */
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import * as THREE from "three";
import { resolveQuality, type BgQualityProfile } from "./quality";

const props = withDefaults(
  defineProps<{
    /** 深色模式（跟随 settings.theme） */
    dark: boolean;
    /** 主题强调色 #RRGGBB，用于夜空银河与水面暖调 */
    accent?: string;
    /** 画质档位；不传时按默认档（均衡） */
    quality?: BgQualityProfile;
  }>(),
  {
    accent: "#f0a63c",
    // 对象类默认值必须用工厂函数，避免所有实例共享同一份 profile
    quality: () => resolveQuality(null),
  },
);

/** 当前画质档位；切换档位由父组件的 :key 触发重建，这里取一次就够 */
const q = props.quality;

const canvasRef = ref<HTMLCanvasElement | null>(null);

/* ------------------------------------------------------------------ *
 * 调色板
 * ------------------------------------------------------------------ */

interface Palette {
  /* 天空 */
  skyTop: number;
  skyLow: number;
  horizon: number;
  sun: number;
  /** 太阳/月亮可见度，夜里压低，避免一个白斑抢画面 */
  sunGlow: number;
  /* 水 */
  deep: number;
  mid: number;
  shallow: number;
  foam: number;
  /* 云 */
  cloudLit: number;
  cloudShadow: number;
  /* 光照 */
  sunLight: number;
  sunIntensity: number;
  hemiSky: number;
  hemiGround: number;
  hemiIntensity: number;
  /** 夜色权重：0 = 白天，1 = 夜晚（驱动星空银河） */
  night: number;
  /** 岛体材质基础色（乘顶点色，夜里整体压暗偏蓝） */
  island: number;
}

/**
 * 白天：天顶压到饱和钴蓝、地平线拉到奶油白，中间只留一档青。
 * 三段而不是一路渐变，就是为了出「色带」而不是「渐变背景」。
 */
const PALETTE_LIGHT: Palette = {
  skyTop: 0x2f7ec9,
  skyLow: 0x8ac5e9,
  horizon: 0xe9f4ef,
  sun: 0xfff3d4,
  sunGlow: 1.0,
  deep: 0x0a4a68,
  mid: 0x1a7a9b,
  shallow: 0x54bdb0,
  foam: 0xf3fdf7,
  cloudLit: 0xffffff,
  cloudShadow: 0x93b4d3,
  sunLight: 0xfff0d6,
  sunIntensity: 3.0,
  hemiSky: 0xbfe4f7,
  hemiGround: 0x8fb08a,
  hemiIntensity: 2.2,
  night: 0,
  island: 0xffffff,
};

const PALETTE_DARK: Palette = {
  skyTop: 0x04070f,
  skyLow: 0x0a1830,
  horizon: 0x1b4058,
  sun: 0xd6e6f5,
  sunGlow: 0.58,
  deep: 0x03101a,
  mid: 0x0a2739,
  shallow: 0x0c3a4e,
  foam: 0x7d9db2,
  cloudLit: 0x3d5c7a,
  cloudShadow: 0x17293c,
  sunLight: 0xcfe0f5,
  sunIntensity: 1.9,
  hemiSky: 0x2b4c6d,
  hemiGround: 0x101f2c,
  hemiIntensity: 2.3,
  night: 1,
  island: 0xcfdcec,
};

/**
 * 太阳方向（世界空间，指向太阳）。
 *
 * 这个位置是画面成败的关键，别随手改：
 * 相机固定在 +x +z 侧看向原点，所以太阳必须落在视锥里（相对视线偏右约 25°、
 * 仰角约 16°），海面才会出现「从地平线一路拉到画面右下」的反光路 —— 二次元
 * 海景最有辨识度的元素。若把太阳挪到相机背后（数据上很好算、岛面也好打光），
 * 天空里看不到太阳，水面就只剩一块没有高光的蓝板子。
 *
 * 代价是岛偏逆光：靠半球光把暗部垫出颜色，不要让环境光太弱。
 */
const SUN_DIR = new THREE.Vector3(-0.203, 0.156, -0.967).normalize();

/* ------------------------------------------------------------------ *
 * 着色器公共片段
 * ------------------------------------------------------------------ */

/** 值噪声。夜空、水面波纹、泡沫、碎光全都靠它，比正弦叠加自然得多 */
const NOISE_FN = /* glsl */ `
float qxHash(vec2 p) {
  // Dave Hoskins 的哈希：sin 版在部分驱动（尤其软件光栅）上精度不够，
  // 会让阈值判定永远为假
  vec3 p3 = fract(vec3(p.xyx) * 0.1031);
  p3 += dot(p3, p3.yzx + 33.33);
  return fract((p3.x + p3.y) * p3.z);
}

float qxNoise(vec2 p) {
  vec2 i = floor(p);
  vec2 f = fract(p);
  f = f * f * (3.0 - 2.0 * f);
  return mix(
    mix(qxHash(i), qxHash(i + vec2(1.0, 0.0)), f.x),
    mix(qxHash(i + vec2(0.0, 1.0)), qxHash(i + vec2(1.0, 1.0)), f.x),
    f.y
  );
}
`;

/**
 * 四向正弦波叠加（只给顶点位移用，控制水面几何的起伏）。
 * 逐像素的波纹不靠它——正弦叠加在屏幕上会显出一眼假的规则网格。
 */
const WAVE_FN = /* glsl */ `
vec3 waveInfo(vec2 p, float t) {
  vec3 acc = vec3(0.0);
  vec2 d0 = normalize(vec2(1.0, 0.34));
  vec2 d1 = normalize(vec2(-0.42, 1.0));
  vec2 d2 = normalize(vec2(1.0, -0.78));
  vec2 d3 = normalize(vec2(0.68, 1.0));
  float f0 = 0.085, f1 = 0.13, f2 = 0.27, f3 = 0.55;
  float a0 = 0.85, a1 = 0.52, a2 = 0.24, a3 = 0.13;
  float s0 = 0.42, s1 = 0.62, s2 = 0.95, s3 = 1.6;

  float ph0 = dot(p, d0) * f0 + t * s0;
  float ph1 = dot(p, d1) * f1 + t * s1;
  float ph2 = dot(p, d2) * f2 + t * s2;
  float ph3 = dot(p, d3) * f3 + t * s3;

  acc.x += sin(ph0) * a0 + sin(ph1) * a1 + sin(ph2) * a2 + sin(ph3) * a3;
  acc.yz += cos(ph0) * a0 * f0 * d0
          + cos(ph1) * a1 * f1 * d1
          + cos(ph2) * a2 * f2 * d2
          + cos(ph3) * a3 * f3 * d3;
  return acc;
}
`;

/* ------------------------------------------------------------------ *
 * 天空
 * ------------------------------------------------------------------ */

const SKY_VERT = /* glsl */ `
varying vec3 vDir;
void main() {
  vDir = position;
  gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
}
`;

const SKY_FRAG = /* glsl */ `
uniform float uTime;
uniform float uNight;
uniform float uSunGlow;
uniform vec3 uSkyTop;
uniform vec3 uSkyLow;
uniform vec3 uHorizon;
uniform vec3 uSunColor;
uniform vec3 uSunDir;
uniform vec3 uAccent;
varying vec3 vDir;

${NOISE_FN}

float hash13(vec3 p3) {
  p3 = fract(p3 * 0.1031);
  p3 += dot(p3, p3.yzx + 33.33);
  return fract((p3.x + p3.y) * p3.z);
}

// 稀疏星点：每格以一定概率放一颗星，格内位置再随机偏移
float starField(vec3 dir) {
  vec3 p = dir * 90.0;
  vec3 cell = floor(p);
  vec3 f = fract(p) - 0.5;
  float pick = hash13(cell);
  if (pick < 0.76) return 0.0;
  vec3 offset = vec3(hash13(cell + 1.3), hash13(cell + 7.7), hash13(cell + 19.1)) - 0.5;
  float dist = length(f - offset * 0.55);
  float mask = 1.0 - smoothstep(0.0, 0.22, dist);
  float twinkle = 0.55 + 0.45 * sin(uTime * 1.6 + pick * 120.0);
  return mask * twinkle * smoothstep(0.76, 0.96, pick);
}

void main() {
  vec3 dir = normalize(vDir);
  float h = dir.y;
  float hh = clamp(h, 0.0, 1.0);

  // 三段分色：地平线奶油白 → 低空青 → 天顶钴蓝。
  // 两段 smoothstep 各自只在很窄的区间内过渡，中间就留出了平色带。
  vec3 col = uHorizon;
  col = mix(col, uSkyLow, smoothstep(0.0, 0.075, hh));
  col = mix(col, uSkyTop, smoothstep(0.05, 0.58, hh));
  // 地平线以下（贴近水面那几度）统一压回雾色，和水面远端无缝对接
  col = mix(uHorizon, col, smoothstep(-0.06, 0.015, h));

  // 太阳方位上的地平线暖光：一条压在低空的横向光带。
  // 少了它，天空就只是「白到下、蓝到上」的渐变纸，一点空气感都没有。
  vec3 hDir = normalize(vec3(dir.x, 0.0, dir.z) + vec3(0.0, 0.0001, 0.0));
  vec3 hSun = normalize(vec3(uSunDir.x, 0.0, uSunDir.z) + vec3(0.0, 0.0001, 0.0));
  float az = pow(clamp(dot(hDir, hSun), 0.0, 1.0), 2.6);
  float lowBand = exp(-pow(h * 10.0, 2.0)) * smoothstep(-0.05, 0.02, h);
  col = mix(col, mix(uSunColor, uHorizon, 0.55), az * lowBand * 0.34 * uSunGlow);

  // 太阳 / 月亮：一个边界清晰的圆盘 + 近晕 + 远晕。
  // 别用单个大指数的高光，那样只会得到一团糊掉的白雾，看不出是「太阳」。
  float sd = max(dot(dir, uSunDir), 0.0);
  float disc = smoothstep(0.99970, 0.99988, sd);
  float halo = pow(sd, 180.0);
  float bloom = pow(sd, 8.0);
  col += uSunColor * disc * 2.9 * uSunGlow;
  col += uSunColor * halo * 0.12 * uSunGlow;
  col += mix(uSunColor, uHorizon, 0.45) * bloom * 0.030 * uSunGlow;

  // 太阳压在地平线上的那层暖光（只在地平线附近出现）
  float low = pow(sd, 7.0) * (1.0 - smoothstep(0.0, 0.26, hh));
  col += uSunColor * low * 0.09 * uSunGlow;

  // 夜空：星点 + 一条被噪声打散的银河。
  // 光带上 exp 的系数必须够大，否则带宽会盖住大半个天球 —— 表现就是整片夜空
  // 被刷上一层灰，什么剪影都没了。
  col += vec3(starField(dir) * smoothstep(0.03, 0.30, h) * uNight) * 0.9;

  vec3 axis = normalize(vec3(0.18, 0.62, 0.76));
  float band = exp(-pow(dot(dir, axis) * 8.0, 2.0));
  float clump = qxNoise(dir.xz * 6.0 + dir.y * 3.0) * 0.65
              + qxNoise(dir.xz * 15.0) * 0.35;
  band *= smoothstep(0.42, 0.85, clump);
  col += mix(uAccent, vec3(0.72, 0.80, 1.0), 0.55) * band * 0.10 * uNight;

  // 提一点饱和度：连续渐变的原始输出天然偏灰，动画背景的蓝要「亮」得多
  col = mix(vec3(dot(col, vec3(0.299, 0.587, 0.114))), col, 1.16);

  gl_FragColor = vec4(col, 1.0);
  #include <colorspace_fragment>
}
`;

/* ------------------------------------------------------------------ *
 * 海面
 * ------------------------------------------------------------------ */

const WATER_VERT = /* glsl */ `
uniform float uTime;
varying vec3 vWorld;
${WAVE_FN}
void main() {
  vec4 world = modelMatrix * vec4(position, 1.0);
  // 起伏幅度压到 ±0.6，并且随距离衰减：远处水面在屏幕上被压成极窄的一条，
  // 等距正弦波会在那儿叠出百叶窗一样的横向条纹。
  float camD = length(world.xz - cameraPosition.xz);
  world.y += waveInfo(world.xz, uTime).x * 0.34 / (1.0 + camD * 0.012);
  vWorld = world.xyz;
  gl_Position = projectionMatrix * viewMatrix * world;
}
`;

const WATER_FRAG = /* glsl */ `
uniform float uTime;
uniform float uNight;
uniform vec3 uDeep;
uniform vec3 uMid;
uniform vec3 uShallow;
uniform vec3 uFoam;
uniform vec3 uHorizon;
uniform vec3 uSkyLow;
uniform vec3 uSunColor;
uniform vec3 uSunDir;
uniform vec3 uAccent;
varying vec3 vWorld;

${NOISE_FN}

// 水面高度场：两层噪声，第二层沿第一层梯度偏移，波纹才有方向感而不是一片雪花。
// 采样坐标在 x 上压扁一点，让波纹沿横向被拉长 —— 这样等值线读起来是「一道道涌浪」，
// 而不是各向同性的噪声贴图。
float seaHeight(vec2 p, float t) {
  vec2 q = p * vec2(0.062, 0.098);
  float a = qxNoise(q + vec2(t * 0.035, t * 0.020));
  float b = qxNoise(q * 2.6 + vec2(a * 1.5 - t * 0.055, t * 0.042));
  return a * 0.66 + b * 0.34;
}

void main() {
  vec2 p = vWorld.xz;
  vec3 toCam = cameraPosition - vWorld;
  float camDist = length(toCam);
  vec3 viewDir = toCam / camDist;
  float t = uTime;

  float hgt = seaHeight(p, t);

  // 中心差分重建法线（只多采两次），远处趋于平坦避免闪烁
  float e = 0.45;
  float hx = seaHeight(p + vec2(e, 0.0), t);
  float hz = seaHeight(p + vec2(0.0, e), t);
  vec3 n = normalize(vec3((hgt - hx) * 2.6, e, (hgt - hz) * 2.6));
  n = normalize(mix(vec3(0.0, 1.0, 0.0), n, 1.0 - smoothstep(60.0, 260.0, camDist)));

  /* ---------- 基础水色 ---------- */

  vec3 col = mix(uDeep, uMid, smoothstep(0.10, 0.92, hgt));

  // 环岛的浅滩：边界用噪声揉碎，别让它看起来是个规整的圆
  float dIsl = length(p);
  float shoalEdge = 18.0 + (qxNoise(p * 0.07 + 5.0) - 0.5) * 12.0;
  float shoal = 1.0 - smoothstep(shoalEdge - 13.0, shoalEdge, dIsl);
  shoal = smoothstep(0.22, 0.78, shoal);   // 收紧过渡带 → 出浅滩色带
  col = mix(col, uShallow, shoal);

  // 岛的倒影：往背光侧偏一点，再用横向拉伸的噪声切碎（波纹会把倒影拉成条）
  vec2 rp = p + uSunDir.xz * 3.5;
  float reflMask = 1.0 - smoothstep(9.0, 20.0, length(rp * vec2(1.0, 1.12)));
  float reflRip = qxNoise(vec2(p.x * 0.55, p.y * 2.8) + vec2(0.0, t * 0.28));
  col *= 1.0 - reflMask * (0.14 + 0.22 * reflRip);

  // 大尺度明暗块面：整片水不能是同一个色，否则就是一块塑料板。
  // 用很低频的噪声压出色块，这是手绘背景表现水体体积的主要手段。
  float slab = qxNoise(p * 0.085 + vec2(t * 0.012, 0.0)) * 0.6
             + qxNoise(p * 0.21 - vec2(0.0, t * 0.018)) * 0.4;
  col *= 0.90 + slab * 0.20;

  // 掠射角的天空反射：越远水面越接近天际色。
  // 强度不能太大 —— 过了的话远海会比地平线上的天空还亮，海天分界直接倒挂。
  float grazing = pow(1.0 - clamp(dot(n, viewDir), 0.0, 1.0), 3.0);
  col = mix(col, mix(uHorizon, uSkyLow, 0.55), grazing * 0.50);

  /* ---------- 波纹笔触 ---------- */

  // 等值线本身是对的，问题是「铺满整片水」就成了大理石纹 / 等高线地形图。
  // 真正的二次元水纹是稀疏的：只出现在波峰、被噪声打断成一段一段的弧线。
  float lineFade = 1.0 - smoothstep(110.0, 330.0, camDist);
  float crest = smoothstep(0.36, 0.92, hgt);
  float cut = smoothstep(0.34, 0.72, qxNoise(p * 0.6 + vec2(t * 0.03, 0.0)));
  float mA = 1.0 - smoothstep(0.0, 0.030, abs(fract(hgt * 4.0) - 0.5));
  float mB = 1.0 - smoothstep(0.0, 0.040, abs(fract(hgt * 9.0 + 0.35) - 0.5));
  // 夜里这几道亮线要收一半：浅滩本身已经够亮，再叠线就成了一片荧光绿
  col += mix(uFoam, uShallow, 0.5)
       * (mA * 0.36 + mB * 0.14)
       * crest * cut * lineFade * (0.32 + 0.68 * shoal) * (1.0 - uNight * 0.5);

  /* ---------- 太阳光路 ---------- */

  vec3 hv = normalize(uSunDir + viewDir);
  float spec = pow(max(dot(n, hv), 0.0), 46.0);
  // 大块反光 + 阈值化成碎片的光斑，两者叠出「碎金」
  col += uSunColor * pow(spec, 5.0) * 0.055;
  float pathMask = smoothstep(0.26, 0.62, spec);
  float sparkN = qxNoise(p * 2.2 + vec2(t * 0.50, -t * 0.30)) * 0.62
               + qxNoise(p * 5.5 - t * 0.70) * 0.38;
  col += mix(uSunColor, uAccent, 0.22) * pathMask * smoothstep(0.55, 0.79, sparkN) * 0.40;

  /* ---------- 岸线泡沫 ---------- */

  // 主泡沫环贴着水线（岛的水线在 r≈11.3，波浪会让它前后摆 ±1）
  float edge = 11.2 + (qxNoise(p * 0.42 + 3.0) - 0.5) * 2.6;
  float foamA = 1.0 - smoothstep(0.0, 1.1, dIsl - edge);
  // 外侧再推一圈更淡的浪，随噪声缓慢推进。别推太远，否则岛周围挂一圈牛奶
  float ring = edge + 1.8 + qxNoise(p * 0.30 + t * 0.10) * 2.0;
  float foamB = (1.0 - smoothstep(0.0, 0.6, abs(dIsl - ring))) * 0.42;
  float breakup = smoothstep(0.34, 0.76, qxNoise(p * 1.3 + t * 0.28));
  float foam = clamp(foamA * 0.92 + foamB, 0.0, 1.0) * (0.5 + 0.5 * breakup);
  // 夜里泡沫要收着点，否则岛周围会挂上一圈刺眼的「牛奶环」
  col = mix(col, uFoam, foam * 0.85 * (1.0 - uNight * 0.55));

  /* ---------- 收尾 ---------- */

  // 距离雾：远处融进「比地平线暗一档的海色」，不要直接融进天空的地平线色 ——
  // 那会把整片远海冲成灰白，海天分界也跟着糊掉
  col = mix(col, mix(uHorizon, uMid, 0.24), smoothstep(320.0, 1500.0, camDist));

  // 轻微色阶量化 + 冷暖分离 + 提饱和，压掉连续渐变的「CG 塑料感」
  col = mix(col, floor(col * 22.0 + 0.5) / 22.0, 0.26);
  float lum = dot(col, vec3(0.299, 0.587, 0.114));
  col *= mix(vec3(0.94, 0.98, 1.06), vec3(1.04, 1.00, 0.94), lum);
  col = mix(vec3(dot(col, vec3(0.299, 0.587, 0.114))), col, 1.14);

  gl_FragColor = vec4(col, 1.0);
  #include <colorspace_fragment>
}
`;

/* ------------------------------------------------------------------ *
 * 云
 * ------------------------------------------------------------------ */

const CLOUD_VERT = /* glsl */ `
varying vec3 vNormalW;
varying vec3 vWorldPos;
void main() {
  vec4 wp = modelMatrix * vec4(position, 1.0);
  vWorldPos = wp.xyz;
  vNormalW = normalize(mat3(modelMatrix) * normal);
  gl_Position = projectionMatrix * viewMatrix * wp;
}
`;

const CLOUD_FRAG = /* glsl */ `
uniform vec3 uLit;
uniform vec3 uShadow;
uniform vec3 uSunColor;
uniform vec3 uSunDir;
varying vec3 vNormalW;
varying vec3 vWorldPos;

void main() {
  vec3 N = normalize(vNormalW);
  vec3 V = normalize(cameraPosition - vWorldPos);
  float ndv = clamp(dot(N, V), 0.0, 1.0);

  // 赛璐璐两色：受光面纯白、底面蓝灰，中间不做第三档。
  // 一旦多出中间色，整团云就显出「球体的立体感」，立刻变成飘在天上的鹅卵石。
  // up 的权重必须大于 lit：相机在云下方，看到的几乎全是云底。
  float lit = dot(N, uSunDir) * 0.5 + 0.5;
  float up = N.y * 0.5 + 0.5;
  float k = lit * 0.40 + up * 0.60;
  float band = smoothstep(0.430, 0.470, k);
  vec3 col = mix(uShadow, uLit, band);

  // 逆光透光：朝向镜头的轮廓亮一圈，云才不像一坨实心橡皮泥
  float rim = pow(1.0 - ndv, 2.6);
  col = mix(col, uLit * 1.06 + uSunColor * 0.04, rim * 0.55);

  gl_FragColor = vec4(col, 1.0);
  #include <colorspace_fragment>
}
`;

/* ------------------------------------------------------------------ *
 * 小岛地形
 * ------------------------------------------------------------------ */

const ISLAND_RADIUS = 13;

/** 岛心到岸边的相对半径 → 高度（u = r / R） */
function heightAt(u: number, ang: number): number {
  const x = Math.cos(ang) * u * ISLAND_RADIUS;
  const z = Math.sin(ang) * u * ISLAND_RADIUS;
  const base = Math.max(0, 1 - Math.pow(u, 1.7));
  let h = 4.6 * Math.pow(base, 1.35) - 1.15 * Math.pow(u, 5.0);
  // 两层低频起伏：大台地 + 小土丘，让平面不像个圆馒头
  const ridge = 0.55 * Math.sin(x * 0.52) * Math.cos(z * 0.58);
  const detail = 0.26 * Math.sin(x * 1.35 + 1.2) * Math.cos(z * 1.05 - 0.6);
  h += (ridge + detail) * Math.min(1, base * 1.7);
  return h;
}

// 顶点色往「动画背景」方向压：沙滩更暖、草地更鲜、岩石偏冷，
// 三个色块之间几乎是硬接的，这就是赛璐璐的读法
const C_SAND = new THREE.Color(0xe8cd78);
const C_SAND_WET = new THREE.Color(0xcbb069);
const C_GRASS = new THREE.Color(0x77a94e);
const C_GRASS_DARK = new THREE.Color(0x55813b);
const C_ROCK = new THREE.Color(0x9aa1a3);

function smoothstep(edge0: number, edge1: number, x: number): number {
  const t = Math.min(1, Math.max(0, (x - edge0) / (edge1 - edge0)));
  return t * t * (3 - 2 * t);
}

const tmpColor = new THREE.Color();
/**
 * 按高度 / 半径给出沙地 → 草地 → 岩石的过渡色（返回线性空间分量）。
 * 沙滩带按半径卡（u 是 r / R），不按高度 —— 高度阈值在岸边给不出足够的宽度，
 * 草地会直接铺到水里，岛就像一块浮着的绿饼。
 */
function surfaceColor(y: number, u: number, wx: number, wz: number): THREE.Color {
  // 岸边坡度很陡，y > 0 的可见部分只有极窄一条，按高度阈值给不出像样的沙滩，
  // 所以沙滩带按半径卡（u = r / R）：外圈 1/4 半径铺沙，草地上沿才接上草。
  const toGrass = 1 - smoothstep(0.70, 0.80, u);
  const toRock = smoothstep(4.3, 5.1, y);
  tmpColor.copy(u > 0.84 ? C_SAND_WET : C_SAND).lerp(C_GRASS, toGrass);
  // 草地上掺一点暗绿斑，避免大片单色
  const patch = smoothstep(0.1, 0.9, Math.sin(wx * 0.9 + 1.1) * Math.cos(wz * 0.8)) * toGrass;
  tmpColor.lerp(C_GRASS_DARK, patch * 0.45).lerp(C_ROCK, toRock);
  // 不要再 convertSRGBToLinear()：THREE.Color 构造时已按 sRGB 解析成线性工作色，
  // 二次转换会把整座岛压暗成一片深棕（沙色最先跑偏）。
  return tmpColor;
}

function buildIslandGeometry(): THREE.BufferGeometry {
  const rSeg = 34;
  const aSeg = 92;
  const positions: number[] = [0, heightAt(0, 0), 0];
  const colors: number[] = [];
  const c0 = surfaceColor(positions[1], 0, 0, 0);
  colors.push(c0.r, c0.g, c0.b);

  for (let j = 1; j <= rSeg; j++) {
    const u = j / rSeg;
    for (let i = 0; i < aSeg; i++) {
      const ang = (i / aSeg) * Math.PI * 2;
      const x = Math.cos(ang) * u * ISLAND_RADIUS;
      const z = Math.sin(ang) * u * ISLAND_RADIUS;
      const y = heightAt(u, ang);
      positions.push(x, y, z);
      const c = surfaceColor(y, u, x, z);
      colors.push(c.r, c.g, c.b);
    }
  }

  const indices: number[] = [];
  // 中心扇形
  for (let i = 0; i < aSeg; i++) {
    indices.push(0, 1 + i, 1 + ((i + 1) % aSeg));
  }
  // 环与环之间
  for (let j = 1; j < rSeg; j++) {
    const inner = 1 + (j - 1) * aSeg;
    const outer = 1 + j * aSeg;
    for (let i = 0; i < aSeg; i++) {
      const i2 = (i + 1) % aSeg;
      indices.push(inner + i, outer + i, outer + i2);
      indices.push(inner + i, outer + i2, inner + i2);
    }
  }

  const geo = new THREE.BufferGeometry();
  geo.setAttribute("position", new THREE.Float32BufferAttribute(positions, 3));
  geo.setAttribute("color", new THREE.Float32BufferAttribute(colors, 3));
  geo.setIndex(indices);
  // flatShading 用导数算法线，但没有 normal 属性时 three 的默认法线管线会拿到零向量
  geo.computeVertexNormals();
  return geo;
}

/** 固定种子的伪随机，保证每次启动的岛是同一座 */
function makeRandom(seed: number): () => number {
  let s = seed >>> 0;
  return () => {
    s = (s * 1664525 + 1013904223) >>> 0;
    return s / 4294967296;
  };
}

function buildIslandMesh(material: THREE.Material): THREE.Group {
  const group = new THREE.Group();
  const land = new THREE.Mesh(buildIslandGeometry(), material);
  group.add(land);

  const rnd = makeRandom(20261114);

  // —— 树 ——
  const trunkMat = new THREE.MeshLambertMaterial({ color: 0x6b4a2f, flatShading: true });
  const leafMat = new THREE.MeshLambertMaterial({ color: 0x3d7a3a, flatShading: true });
  const leafMatAlt = new THREE.MeshLambertMaterial({ color: 0x4f9142, flatShading: true });
  const trunkGeo = new THREE.CylinderGeometry(0.14, 0.22, 1.1, 5);
  const coneGeos = [1.15, 0.9, 0.62].map((r) => new THREE.ConeGeometry(r, 1.25, 6));

  let planted = 0;
  for (let attempt = 0; attempt < 220 && planted < 11; attempt++) {
    const u = 0.16 + rnd() * 0.56;
    const ang = rnd() * Math.PI * 2;
    const y = heightAt(u, ang);
    if (y < 1.15) continue;

    const tree = new THREE.Group();
    const trunk = new THREE.Mesh(trunkGeo, trunkMat);
    trunk.position.y = 0.5;
    tree.add(trunk);
    coneGeos.forEach((geo, k) => {
      const cone = new THREE.Mesh(geo, k % 2 ? leafMatAlt : leafMat);
      cone.position.y = 1.25 + k * 0.62;
      tree.add(cone);
    });
    const s = 0.72 + rnd() * 0.55;
    tree.scale.setScalar(s);
    tree.position.set(Math.cos(ang) * u * ISLAND_RADIUS, y - 0.12, Math.sin(ang) * u * ISLAND_RADIUS);
    tree.rotation.y = rnd() * Math.PI * 2;
    group.add(tree);
    planted++;
  }

  // —— 礁石 ——
  const rockMat = new THREE.MeshLambertMaterial({ color: 0x93999b, flatShading: true });
  const rockGeo = new THREE.IcosahedronGeometry(0.55, 0);
  for (let i = 0; i < 7; i++) {
    const u = 0.55 + rnd() * 0.4;
    const ang = rnd() * Math.PI * 2;
    const y = heightAt(u, ang);
    const rock = new THREE.Mesh(rockGeo, rockMat);
    rock.position.set(Math.cos(ang) * u * ISLAND_RADIUS, y + 0.1, Math.sin(ang) * u * ISLAND_RADIUS);
    rock.scale.set(0.7 + rnd() * 1.5, 0.5 + rnd() * 1.1, 0.7 + rnd() * 1.4);
    rock.rotation.set(rnd() * 3, rnd() * 3, rnd() * 3);
    group.add(rock);
  }

  return group;
}

/* ------------------------------------------------------------------ *
 * 场景装配与生命周期
 * ------------------------------------------------------------------ */

let renderer: THREE.WebGLRenderer | null = null;
let scene: THREE.Scene | null = null;
let camera: THREE.PerspectiveCamera | null = null;
let skyMat: THREE.ShaderMaterial | null = null;
let waterMat: THREE.ShaderMaterial | null = null;
let cloudMat: THREE.ShaderMaterial | null = null;
let sunLight: THREE.DirectionalLight | null = null;
let hemiLight: THREE.HemisphereLight | null = null;
let islandMat: THREE.MeshLambertMaterial | null = null;
let cloudGroup: THREE.Group | null = null;
let cloudBases: { x: number; y: number; z: number; speed: number }[] = [];

let raf = 0;
let startedAt = 0;
let lastFrameAt = 0;
let active = false;
const pointer = { x: 0, y: 0, tx: 0, ty: 0 };

function applyPalette(dark: boolean) {
  const p = dark ? PALETTE_DARK : PALETTE_LIGHT;
  const accent = new THREE.Color(props.accent || "#f0a63c");

  if (skyMat) {
    skyMat.uniforms.uSkyTop.value.setHex(p.skyTop);
    skyMat.uniforms.uSkyLow.value.setHex(p.skyLow);
    skyMat.uniforms.uHorizon.value.setHex(p.horizon);
    skyMat.uniforms.uSunColor.value.setHex(p.sun);
    skyMat.uniforms.uSunGlow.value = p.sunGlow;
    skyMat.uniforms.uSunDir.value.copy(SUN_DIR);
    skyMat.uniforms.uAccent.value.copy(accent);
    skyMat.uniforms.uNight.value = p.night;
  }
  if (waterMat) {
    waterMat.uniforms.uDeep.value.setHex(p.deep);
    waterMat.uniforms.uMid.value.setHex(p.mid);
    waterMat.uniforms.uShallow.value.setHex(p.shallow);
    waterMat.uniforms.uFoam.value.setHex(p.foam);
    waterMat.uniforms.uHorizon.value.setHex(p.horizon);
    waterMat.uniforms.uSkyLow.value.setHex(p.skyLow);
    waterMat.uniforms.uSunColor.value.setHex(p.sun);
    waterMat.uniforms.uSunDir.value.copy(SUN_DIR);
    waterMat.uniforms.uAccent.value.copy(accent);
    waterMat.uniforms.uNight.value = p.night;
  }
  if (cloudMat) {
    cloudMat.uniforms.uLit.value.setHex(p.cloudLit);
    cloudMat.uniforms.uShadow.value.setHex(p.cloudShadow);
    cloudMat.uniforms.uSunColor.value.setHex(p.sun);
    cloudMat.uniforms.uSunDir.value.copy(SUN_DIR);
  }
  if (sunLight) {
    sunLight.color.setHex(p.sunLight);
    sunLight.intensity = p.sunIntensity;
  }
  if (hemiLight) {
    hemiLight.color.setHex(p.hemiSky);
    hemiLight.groundColor.setHex(p.hemiGround);
    hemiLight.intensity = p.hemiIntensity;
  }
  if (islandMat) islandMat.color.setHex(p.island);
}

function resize() {
  if (!renderer || !camera) return;
  const host = canvasRef.value?.parentElement ?? null;
  const w = Math.max(1, host?.clientWidth || window.innerWidth);
  const h = Math.max(1, host?.clientHeight || window.innerHeight);
  renderer.setSize(w, h, false);
  camera.aspect = w / h;
  camera.updateProjectionMatrix();
  // 静止模式（减少动态效果）下没有动画循环，尺寸变化后要自己补一帧
  if (!active) {
    step(12);
    renderOnce();
  }
}

function onPointerMove(e: PointerEvent) {
  pointer.tx = (e.clientX / window.innerWidth - 0.5) * 2;
  pointer.ty = (e.clientY / window.innerHeight - 0.5) * 2;
}

function step(t: number) {
  if (waterMat) waterMat.uniforms.uTime.value = t;
  if (skyMat) skyMat.uniforms.uTime.value = t;

  // 指针视差：慢速跟随，避免鼠标一动画面就晃
  pointer.x += (pointer.tx - pointer.x) * 0.04;
  pointer.y += (pointer.ty - pointer.y) * 0.04;

  if (camera) {
    // 极缓慢的推拉 + 横向漂移，像一段循环的空镜
    camera.position.set(
      23 + Math.sin(t * 0.042) * 2.4 + pointer.x * 2.8,
      9.8 + Math.sin(t * 0.071) * 0.75 - pointer.y * 1.5,
      31 + Math.cos(t * 0.036) * 1.8,
    );
    camera.lookAt(0, 3.6 + Math.sin(t * 0.055) * 0.32, 0);
  }

  // 云横向漂移，出界后从另一侧回来
  if (cloudGroup) {
    cloudGroup.children.forEach((cloud, i) => {
      const base = cloudBases[i];
      if (!base) return;
      const span = 260;
      let x = base.x + t * base.speed;
      if (x > span) x -= span * 2;
      cloud.position.set(x, base.y + Math.sin(t * 0.15 + i) * 0.6, base.z);
    });
  }
}

function renderOnce() {
  if (renderer && scene && camera) renderer.render(scene, camera);
}

function loop(now: number) {
  raf = requestAnimationFrame(loop);
  if (!active || document.hidden) return;
  // 帧率上限跟着画质档：流畅档 30fps 就够，镜头运动本来就很慢。
  // 留 1.5ms 余量，否则 60 档会因 rAF 间隔抖动退化成 30fps
  if (now - lastFrameAt < 1000 / q.fps - 1.5) return;
  lastFrameAt = now;
  step((now - startedAt) / 1000);
  renderOnce();
}

function init() {
  const canvas = canvasRef.value;
  if (!canvas) return;

  try {
    renderer = new THREE.WebGLRenderer({
      canvas,
      // 低模场景锯齿明显，但流畅档优先保帧率
      antialias: q.level >= 1,
      powerPreference: q.level === 0 ? "low-power" : "default",
    });
  } catch (err) {
    // 显卡/DirectX 不可用：留空，父层渐变背景照常显示
    console.warn("[background] WebGL 不可用，动态背景降级为渐变底色：", err);
    canvas.style.display = "none";
    return;
  }

  renderer.setPixelRatio(q.pixelRatio(window.devicePixelRatio || 1));
  renderer.setClearColor(0x000000, 1);

  scene = new THREE.Scene();
  camera = new THREE.PerspectiveCamera(40, 1, 0.5, 9000);
  camera.position.set(23, 9.8, 31);
  camera.lookAt(0, 3.6, 0);

  // —— 天空穹顶 ——
  skyMat = new THREE.ShaderMaterial({
    side: THREE.BackSide,
    depthWrite: false,
    uniforms: {
      uTime: { value: 0 },
      uNight: { value: 1 },
      uSunGlow: { value: 1 },
      uSkyTop: { value: new THREE.Color() },
      uSkyLow: { value: new THREE.Color() },
      uHorizon: { value: new THREE.Color() },
      uSunColor: { value: new THREE.Color() },
      uSunDir: { value: SUN_DIR.clone() },
      uAccent: { value: new THREE.Color(props.accent || "#f0a63c") },
    },
    vertexShader: SKY_VERT,
    fragmentShader: SKY_FRAG,
  });
  const sky = new THREE.Mesh(new THREE.SphereGeometry(4200, 32, 24), skyMat);
  sky.renderOrder = -1;
  scene.add(sky);

  // —— 海面 ——
  // 水面顶点位移靠几何细分，档位低就少切几刀（顶点数按平方下降）
  const waterSeg = q.level === 2 ? 180 : q.level === 1 ? 150 : 96;
  const waterGeo = new THREE.PlaneGeometry(2400, 2400, waterSeg, waterSeg);
  waterGeo.rotateX(-Math.PI / 2);
  waterMat = new THREE.ShaderMaterial({
    uniforms: {
      uTime: { value: 0 },
      uNight: { value: 1 },
      uDeep: { value: new THREE.Color() },
      uMid: { value: new THREE.Color() },
      uShallow: { value: new THREE.Color() },
      uFoam: { value: new THREE.Color() },
      uHorizon: { value: new THREE.Color() },
      uSkyLow: { value: new THREE.Color() },
      uSunColor: { value: new THREE.Color() },
      uSunDir: { value: SUN_DIR.clone() },
      uAccent: { value: new THREE.Color(props.accent || "#f0a63c") },
    },
    vertexShader: WATER_VERT,
    fragmentShader: WATER_FRAG,
  });
  scene.add(new THREE.Mesh(waterGeo, waterMat));

  // —— 小岛 ——
  islandMat = new THREE.MeshLambertMaterial({
    vertexColors: true,
    flatShading: true,
    side: THREE.DoubleSide,
  });
  scene.add(buildIslandMesh(islandMat));

  // —— 云 ——
  // 不透明 + 赛璐璐明暗。之前用 MeshBasicMaterial 纯色球，完全没明暗，
  // 云就成了贴在天上的一堆色块，这是「塑料感」最主要的来源之一。
  cloudMat = new THREE.ShaderMaterial({
    uniforms: {
      uLit: { value: new THREE.Color() },
      uShadow: { value: new THREE.Color() },
      uSunColor: { value: new THREE.Color() },
      uSunDir: { value: SUN_DIR.clone() },
    },
    vertexShader: CLOUD_VERT,
    fragmentShader: CLOUD_FRAG,
  });

  cloudGroup = new THREE.Group();
  const cloudRnd = makeRandom(77123);
  const puffGeo = new THREE.IcosahedronGeometry(1, q.level >= 1 ? 2 : 1);
  cloudBases = [];
  const cloudCount = q.level >= 1 ? 11 : 8;

  for (let i = 0; i < cloudCount; i++) {
    const cloud = new THREE.Group();
    // 越远越小：用 z 反推缩放，云层才有纵深，不再是一排等大的棉花糖
    const depth = 0.35 + cloudRnd() * 0.65;           // 0.35 近 → 1.0 远
    const sizeK = 1.15 - depth * 0.62;
    const puffs = 7 + Math.floor(cloudRnd() * 3);

    // 主体：用一串大小不一的小球铺开，而不是几个大球。
    // 球一大，外轮廓就是一个闭合的光滑椭圆 —— 那正是「鹅卵石」的来源；
    // 球小而密，轮廓才有扇贝状的起伏。
    for (let k = 0; k < puffs; k++) {
      const f = (k / (puffs - 1) - 0.5) * 2;
      const crown = 1 - Math.pow(Math.abs(f), 1.6);   // 中间高、两端低
      const s = (7.0 + cloudRnd() * 5.0) * (0.55 + crown * 0.75);
      const puff = new THREE.Mesh(puffGeo, cloudMat);
      puff.position.set(
        f * 21 + (cloudRnd() - 0.5) * 6.0,
        crown * 5.5 + (cloudRnd() - 0.5) * 2.4,
        (cloudRnd() - 0.5) * 9.0,
      );
      puff.scale.set(s, s * 0.78, s * 0.86);
      cloud.add(puff);
    }
    // 顶上再堆几团，把轮廓推出花椰菜似的凸起
    for (let k = 0; k < 4; k++) {
      const s = 5.0 + cloudRnd() * 4.0;
      const puff = new THREE.Mesh(puffGeo, cloudMat);
      puff.position.set(
        (cloudRnd() - 0.5) * 26,
        8.0 + cloudRnd() * 6.0,
        (cloudRnd() - 0.5) * 8.0,
      );
      puff.scale.set(s, s * 0.86, s * 0.9);
      cloud.add(puff);
    }

    cloud.scale.setScalar(sizeK * (0.85 + cloudRnd() * 0.3));
    const base = {
      x: -300 + cloudRnd() * 600,
      y: 14 + cloudRnd() * 32,
      z: -150 - cloudRnd() * 330,
      speed: 0.30 + cloudRnd() * 0.55,
    };
    cloud.position.set(base.x, base.y, base.z);
    cloudGroup.add(cloud);
    cloudBases.push(base);
  }
  scene.add(cloudGroup);

  // —— 光照 ——
  // 逆光构图的补偿：方向光给轮廓，半球光负责把暗部垫成有颜色的灰，而不是死黑
  sunLight = new THREE.DirectionalLight(0xffffff, 3.0);
  sunLight.position.copy(SUN_DIR).multiplyScalar(220);
  scene.add(sunLight);
  hemiLight = new THREE.HemisphereLight(0xffffff, 0x444444, 2.2);
  scene.add(hemiLight);

  applyPalette(props.dark);
  resize();
  window.addEventListener("pointermove", onPointerMove, { passive: true });
  window.addEventListener("resize", resize);

  const reduced = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
  if (reduced) {
    // 减少动态：给一帧静止画面，不启动动画循环
    step(12);
    renderOnce();
    return;
  }

  active = true;
  startedAt = performance.now();
  lastFrameAt = 0;
  raf = requestAnimationFrame(loop);
}

function destroy() {
  cancelAnimationFrame(raf);
  raf = 0;
  active = false;
  window.removeEventListener("pointermove", onPointerMove);
  window.removeEventListener("resize", resize);

  scene?.traverse((obj) => {
    const mesh = obj as THREE.Mesh;
    if (mesh.isMesh) {
      mesh.geometry?.dispose();
      const mat = mesh.material as THREE.Material | THREE.Material[] | undefined;
      if (Array.isArray(mat)) mat.forEach((m) => m.dispose());
      else mat?.dispose();
    }
  });
  scene?.clear();
  scene = null;
  renderer?.dispose();
  // 主动丢上下文：频繁开关背景模式时避免 WebGL context 堆积被浏览器回收其它画布
  renderer?.forceContextLoss();
  renderer = null;
  camera = null;
  skyMat = null;
  waterMat = null;
  cloudMat = null;
  sunLight = null;
  hemiLight = null;
  islandMat = null;
  cloudGroup = null;
  cloudBases = [];
}

onMounted(init);
onBeforeUnmount(destroy);

watch(
  () => props.dark,
  (dark) => {
    applyPalette(dark);
    if (!active) {
      step(12);
      renderOnce();
    }
  },
);

watch(
  () => props.accent,
  () => applyPalette(props.dark),
);
</script>

<template>
  <canvas ref="canvasRef" class="island-scene" aria-hidden="true"></canvas>
</template>

<style scoped>
.island-scene {
  position: absolute;
  inset: 0;
  display: block;
  width: 100%;
  height: 100%;
  pointer-events: none;
}
</style>
