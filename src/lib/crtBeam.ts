// オシロスコープの CRT ビーム描画 (WebGL2)。FluentScope Next の crt_renderer.py を移植したもの。
//
// ビームのモデル:
//   - Oscilloscope by Hansi Raber (https://github.com/kritzikratzi/Oscilloscope, MIT License,
//     (c) 2017 hansi raber): bin/data/shaders/osci.frag, src/OsciMesh.cpp
//   - woscope by m1el (https://m1el.github.io/woscope-how/): 電子ビームの物理ベースの描画
//
// サンプル間の 1 ステップごとに、線分のまわりの四角形を描く。フラグメントシェーダーは、1 ステップの間に
// 等速で動くガウス分布のスポットの積分を閉じた式で求める:
//   I(p) = E σ / (2 l) · exp(-py² / 2σ²) · (erf(px / √2σ) - erf((px - l) / √2σ))
// どのステップも同じエネルギー E を置くので、ゆっくり動く部分は明るく、速い縁は暗くなる。
// 浮動小数の蛍光面バッファに足し込み、表示の更新ごとに exp(-dt / 残光) を掛けて減衰させる。
// 2 段のブルーム (1/2・1/4 解像度のぼかし) で光のにじみを付け、表示ガンマと飽和の膝 (明るすぎる部分は白へ) を通す。

const BEAM_VERT = `#version 300 es
precision highp float;
uniform vec2 uViewport;
uniform float uSigma;
layout(location = 0) in vec4 aSeg;
layout(location = 1) in float aEnergy;
out vec3 vUVL;
out float vEnergy;
void main() {
  int id = gl_VertexID & 3;
  vec2 p0 = aSeg.xy;
  vec2 p1 = aSeg.zw;
  vec2 d = p1 - p0;
  float len = length(d);
  vec2 dir = len > 1e-5 ? d / len : vec2(1.0, 0.0);
  vec2 nrm = vec2(-dir.y, dir.x);
  float r = uSigma * 4.0;
  float side = (id & 1) == 0 ? -1.0 : 1.0;
  bool atEnd = id >= 2;
  vec2 p = (atEnd ? p1 + dir * r : p0 - dir * r) + nrm * (side * r);
  vUVL = vec3(atEnd ? len + r : -r, side * r, len);
  vEnergy = aEnergy;
  vec2 ndc = p / uViewport * 2.0 - 1.0;
  gl_Position = vec4(ndc.x, -ndc.y, 0.0, 1.0);
}`;

const BEAM_FRAG = `#version 300 es
precision highp float;
#define SQRT2 1.4142135623730951
#define INV_SQRT_TAU 0.3989422804014327
uniform float uSigma;
uniform vec3 uColor;
in vec3 vUVL;
in float vEnergy;
out vec4 fragColor;
float erfAs(float x) {
  float s = sign(x);
  float a = abs(x);
  float t = 1.0 / (1.0 + 0.3275911 * a);
  float y = 1.0 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t * exp(-a * a);
  return s * y;
}
void main() {
  float len = vUVL.z;
  vec2 xy = vUVL.xy;
  float s = uSigma;
  float gy = exp(-xy.y * xy.y / (2.0 * s * s));
  float a;
  if (len < 1e-3 * s) {
    a = exp(-xy.x * xy.x / (2.0 * s * s)) * gy * INV_SQRT_TAU;
  } else {
    a = (erfAs(xy.x / (SQRT2 * s)) - erfAs((xy.x - len) / (SQRT2 * s))) * gy * s / (2.0 * len);
  }
  fragColor = vec4(uColor * (a * vEnergy), 1.0);
}`;

const FULLSCREEN_VERT = `#version 300 es
out vec2 vUV;
void main() {
  vec2 p = vec2((gl_VertexID << 1) & 2, gl_VertexID & 2);
  vUV = p;
  gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}`;

const FADE_FRAG = `#version 300 es
precision highp float;
uniform float uDecay;
out vec4 fragColor;
void main() { fragColor = vec4(vec3(uDecay), 1.0); }`;

const DOWNSAMPLE_FRAG = `#version 300 es
precision highp float;
uniform sampler2D uSrc;
in vec2 vUV;
out vec4 fragColor;
void main() {
  vec2 t = 1.0 / vec2(textureSize(uSrc, 0));
  vec3 c = texture(uSrc, vUV + t * vec2(-1.0, -1.0)).rgb
         + texture(uSrc, vUV + t * vec2( 1.0, -1.0)).rgb
         + texture(uSrc, vUV + t * vec2(-1.0,  1.0)).rgb
         + texture(uSrc, vUV + t * vec2( 1.0,  1.0)).rgb;
  fragColor = vec4(c * 0.25, 1.0);
}`;

const BLUR_FRAG = `#version 300 es
precision highp float;
uniform sampler2D uSrc;
uniform vec2 uDir;
in vec2 vUV;
out vec4 fragColor;
void main() {
  vec2 t = uDir / vec2(textureSize(uSrc, 0));
  vec3 c = texture(uSrc, vUV).rgb * 0.2270270270;
  c += texture(uSrc, vUV + t * 1.3846153846).rgb * 0.3162162162;
  c += texture(uSrc, vUV - t * 1.3846153846).rgb * 0.3162162162;
  c += texture(uSrc, vUV + t * 3.2307692308).rgb * 0.0702702703;
  c += texture(uSrc, vUV - t * 3.2307692308).rgb * 0.0702702703;
  fragColor = vec4(c, 1.0);
}`;

const COMPOSITE_FRAG = `#version 300 es
precision highp float;
uniform sampler2D uHDR;
uniform sampler2D uBloomNear;
uniform sampler2D uBloomFar;
uniform float uExposure;
uniform float uBloomNearStr;
uniform float uBloomFarStr;
uniform float uWhite;
uniform float uGamma;
uniform float uKnee;
in vec2 vUV;
out vec4 fragColor;
float gammaSaturate(float L, out float v) {
  v = pow(max(L, 0.0), 1.0 / uGamma);
  return v / pow(1.0 + pow(v, uKnee), 1.0 / uKnee);
}
void main() {
  vec3 hdr = texture(uHDR, vUV).rgb
           + texture(uBloomNear, vUV).rgb * uBloomNearStr
           + texture(uBloomFar, vUV).rgb * uBloomFarStr;
  hdr = max(hdr, vec3(0.0)) * uExposure;
  float peak = max(hdr.r, max(hdr.g, hdr.b));
  vec3 hue = peak > 1e-6 ? hdr / peak : vec3(0.0);
  float v;
  float b = gammaSaturate(peak, v);
  vec3 beam = mix(hue, vec3(1.0), uWhite * smoothstep(1.0, 4.0, v)) * b;
  // 黒地の上の光 (下の目盛りへは CSS の screen 合成で重ねる)
  fragColor = vec4(beam, 1.0);
}`;

type Target = { tex: WebGLTexture; fbo: WebGLFramebuffer; w: number; h: number };

export class CrtBeam {
  // 調整値
  sigma = 1.3; // ビームのスポットの標準偏差 [CSS px]
  exposure = 2.5;
  /** ブルーム (光のにじみ)。切っているときはぼかしの処理もしない */
  bloom = false;
  bloomNear = 0.6;
  bloomFar = 0.35;
  whiteCore = 0.6;
  gamma = 2.2;
  knee = 5.0;
  color: [number, number, number] = [0.35, 1.0, 0.45];

  private gl: WebGL2RenderingContext;
  private beamProg: WebGLProgram;
  private fadeProg: WebGLProgram;
  private downProg: WebGLProgram;
  private blurProg: WebGLProgram;
  private compProg: WebGLProgram;
  private beamVao: WebGLVertexArrayObject;
  private emptyVao: WebGLVertexArrayObject;
  private segBuf: WebGLBuffer;
  private segCap = 0;
  private hdr: Target | null = null;
  private bloomTargets: [Target, Target][] = [];
  private hdrFormat: { internal: number; type: number };
  private width = 0;
  private height = 0;
  /** 1 CSS px が何画素か */
  scale = 1;

  constructor(private canvas: HTMLCanvasElement) {
    const gl = canvas.getContext("webgl2", { premultipliedAlpha: false, antialias: false, alpha: false });
    if (!gl) throw new Error("WebGL2 が使えません");
    this.gl = gl;
    // 浮動小数の描画先 (半精度は EXT_color_buffer_float があれば描けて足し込みもできる)
    const floatOk = !!gl.getExtension("EXT_color_buffer_float");
    gl.getExtension("EXT_float_blend");
    this.hdrFormat = floatOk ? { internal: gl.RGBA16F, type: gl.HALF_FLOAT } : { internal: gl.RGBA8, type: gl.UNSIGNED_BYTE };
    this.beamProg = this.program(BEAM_VERT, BEAM_FRAG);
    this.fadeProg = this.program(FULLSCREEN_VERT, FADE_FRAG);
    this.downProg = this.program(FULLSCREEN_VERT, DOWNSAMPLE_FRAG);
    this.blurProg = this.program(FULLSCREEN_VERT, BLUR_FRAG);
    this.compProg = this.program(FULLSCREEN_VERT, COMPOSITE_FRAG);
    this.emptyVao = gl.createVertexArray()!;
    this.beamVao = gl.createVertexArray()!;
    this.segBuf = gl.createBuffer()!;
    gl.bindVertexArray(this.beamVao);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.segBuf);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 4, gl.FLOAT, false, 20, 0);
    gl.vertexAttribDivisor(0, 1);
    gl.enableVertexAttribArray(1);
    gl.vertexAttribPointer(1, 1, gl.FLOAT, false, 20, 16);
    gl.vertexAttribDivisor(1, 1);
    gl.bindVertexArray(null);
  }

  private program(vs: string, fs: string): WebGLProgram {
    const gl = this.gl;
    const sh = (type: number, src: string) => {
      const s = gl.createShader(type)!;
      gl.shaderSource(s, src);
      gl.compileShader(s);
      if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s) ?? "shader");
      return s;
    };
    const p = gl.createProgram()!;
    gl.attachShader(p, sh(gl.VERTEX_SHADER, vs));
    gl.attachShader(p, sh(gl.FRAGMENT_SHADER, fs));
    gl.linkProgram(p);
    if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(p) ?? "link");
    return p;
  }

  private target(w: number, h: number, internal: number, type: number): Target {
    const gl = this.gl;
    const tex = gl.createTexture()!;
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.texImage2D(gl.TEXTURE_2D, 0, internal, w, h, 0, gl.RGBA, type, null);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    const fbo = gl.createFramebuffer()!;
    gl.bindFramebuffer(gl.FRAMEBUFFER, fbo);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0);
    return { tex, fbo, w, h };
  }

  private release(t: Target | null) {
    if (!t) return;
    this.gl.deleteFramebuffer(t.fbo);
    this.gl.deleteTexture(t.tex);
  }

  /** 表示の大きさに合わせる (変わったら蛍光面を消す)。戻り値は作り直したか */
  resize(): boolean {
    const dpr = window.devicePixelRatio || 1;
    const w = Math.max(1, Math.round(this.canvas.clientWidth * dpr));
    const h = Math.max(1, Math.round(this.canvas.clientHeight * dpr));
    this.scale = dpr;
    if (w === this.width && h === this.height && this.hdr) return false;
    this.canvas.width = w;
    this.canvas.height = h;
    this.width = w;
    this.height = h;
    this.release(this.hdr);
    for (const [a, b] of this.bloomTargets) {
      this.release(a);
      this.release(b);
    }
    const { internal, type } = this.hdrFormat;
    this.hdr = this.target(w, h, internal, type);
    this.bloomTargets = [2, 4].map((div) => {
      const bw = Math.max(1, Math.floor(w / div)),
        bh = Math.max(1, Math.floor(h / div));
      return [this.target(bw, bh, internal, type), this.target(bw, bh, internal, type)] as [Target, Target];
    });
    this.clear();
    return true;
  }

  /** 蛍光面を消す */
  clear() {
    const gl = this.gl;
    for (const t of [this.hdr, ...this.bloomTargets.flat()]) {
      if (!t) continue;
      gl.bindFramebuffer(gl.FRAMEBUFFER, t.fbo);
      gl.viewport(0, 0, t.w, t.h);
      gl.clearColor(0, 0, 0, 0);
      gl.clear(gl.COLOR_BUFFER_BIT);
    }
  }

  /** 蛍光面を `decay` 倍に減衰させる (exp(-dt / 残光)) */
  fade(decay: number) {
    const gl = this.gl;
    if (!this.hdr) return;
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.hdr.fbo);
    gl.viewport(0, 0, this.width, this.height);
    if (decay <= 0) {
      gl.clearColor(0, 0, 0, 0);
      gl.clear(gl.COLOR_BUFFER_BIT);
      return;
    }
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.ZERO, gl.SRC_COLOR);
    gl.useProgram(this.fadeProg);
    gl.uniform1f(gl.getUniformLocation(this.fadeProg, "uDecay"), decay);
    gl.bindVertexArray(this.emptyVao);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
  }

  /**
   * ビームを描く。`segs` は (x0, y0, x1, y1, energy) × n (画素、y は下向き)。
   */
  drawBeam(segs: Float32Array, n: number) {
    const gl = this.gl;
    if (!this.hdr || n <= 0) return;
    gl.bindBuffer(gl.ARRAY_BUFFER, this.segBuf);
    if (n > this.segCap) {
      this.segCap = Math.max(n, this.segCap * 2, 16384);
      gl.bufferData(gl.ARRAY_BUFFER, this.segCap * 20, gl.DYNAMIC_DRAW);
    }
    gl.bufferSubData(gl.ARRAY_BUFFER, 0, segs, 0, n * 5);
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.hdr.fbo);
    gl.viewport(0, 0, this.width, this.height);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.ONE, gl.ONE);
    gl.useProgram(this.beamProg);
    gl.uniform2f(gl.getUniformLocation(this.beamProg, "uViewport"), this.width, this.height);
    gl.uniform1f(gl.getUniformLocation(this.beamProg, "uSigma"), this.sigma * this.scale);
    gl.uniform3f(gl.getUniformLocation(this.beamProg, "uColor"), ...this.color);
    gl.bindVertexArray(this.beamVao);
    gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, n);
    gl.bindVertexArray(null);
  }

  private fullscreen(prog: WebGLProgram, dst: Target | null) {
    const gl = this.gl;
    gl.bindFramebuffer(gl.FRAMEBUFFER, dst ? dst.fbo : null);
    gl.viewport(0, 0, dst ? dst.w : this.width, dst ? dst.h : this.height);
    gl.useProgram(prog);
    gl.bindVertexArray(this.emptyVao);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
  }

  /** ブルームを掛けて画面へ出す */
  present() {
    const gl = this.gl;
    if (!this.hdr) return;
    gl.disable(gl.BLEND);
    let src = this.hdr;
    for (const [a, b] of this.bloom ? this.bloomTargets : []) {
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, src.tex);
      gl.useProgram(this.downProg);
      gl.uniform1i(gl.getUniformLocation(this.downProg, "uSrc"), 0);
      this.fullscreen(this.downProg, a);
      for (const [dir, from, to] of [
        [[1, 0], a, b],
        [[0, 1], b, a],
      ] as [[number, number], Target, Target][]) {
        gl.bindTexture(gl.TEXTURE_2D, from.tex);
        gl.useProgram(this.blurProg);
        gl.uniform1i(gl.getUniformLocation(this.blurProg, "uSrc"), 0);
        gl.uniform2f(gl.getUniformLocation(this.blurProg, "uDir"), dir[0], dir[1]);
        this.fullscreen(this.blurProg, to);
      }
      src = a;
    }
    const p = this.compProg;
    gl.useProgram(p);
    const tex = [this.hdr.tex, this.bloomTargets[0][0].tex, this.bloomTargets[1][0].tex];
    ["uHDR", "uBloomNear", "uBloomFar"].forEach((name, i) => {
      gl.activeTexture(gl.TEXTURE0 + i);
      gl.bindTexture(gl.TEXTURE_2D, tex[i]);
      gl.uniform1i(gl.getUniformLocation(p, name), i);
    });
    gl.uniform1f(gl.getUniformLocation(p, "uExposure"), this.exposure);
    gl.uniform1f(gl.getUniformLocation(p, "uBloomNearStr"), this.bloom ? this.bloomNear : 0);
    gl.uniform1f(gl.getUniformLocation(p, "uBloomFarStr"), this.bloom ? this.bloomFar : 0);
    gl.uniform1f(gl.getUniformLocation(p, "uWhite"), this.whiteCore);
    gl.uniform1f(gl.getUniformLocation(p, "uGamma"), this.gamma);
    gl.uniform1f(gl.getUniformLocation(p, "uKnee"), this.knee);
    this.fullscreen(p, null);
    gl.activeTexture(gl.TEXTURE0);
  }

  dispose() {
    const gl = this.gl;
    this.release(this.hdr);
    for (const [a, b] of this.bloomTargets) {
      this.release(a);
      this.release(b);
    }
    gl.deleteBuffer(this.segBuf);
    gl.getExtension("WEBGL_lose_context")?.loseContext();
  }
}
