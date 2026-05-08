precision highp float;

// Palette
uniform vec3 uColorDeep;   // #0a0f1a -- valleys
uniform vec3 uColorMid;    // #3a2030 -- rose-deep
uniform vec3 uColorHigh;   // #dca5bd -- rose-glow
uniform vec3 uColorPeak;   // #d8c8a0 -- bone-bright

// Fog
uniform vec3 uFogColor;    // #060608 -- void
uniform float uFogDensity;

// Hash-derived modulation
uniform float uHueShift;   // 0-1
uniform float uSaturation; // 0-1
uniform float uOpacity;

varying float vHeight;
varying float vFogDepth;
varying vec2 vUv;

// Hue rotation (Rodrigues)
vec3 hueRotate(vec3 color, float shift) {
  float angle = (shift - 0.5) * 0.3 * 6.2832;
  float s = sin(angle);
  float c = cos(angle);
  vec3 w = vec3(0.299, 0.587, 0.114);
  float dot_val = dot(color, w);
  vec3 rotated;
  rotated.r = dot_val + (color.r - dot_val) * c + (0.168 * color.r - 0.131 * color.g - 0.037 * color.b) * s;
  rotated.g = dot_val + (color.g - dot_val) * c + (0.330 * color.r + 0.174 * color.g - 0.504 * color.b) * s;
  rotated.b = dot_val + (color.b - dot_val) * c + (-0.497 * color.r + 0.429 * color.g + 0.068 * color.b) * s;
  return rotated;
}

void main() {
  // Height-based color gradient: 4-stop
  vec3 color;
  if (vHeight < 0.33) {
    color = mix(uColorDeep, uColorMid, vHeight / 0.33);
  } else if (vHeight < 0.66) {
    color = mix(uColorMid, uColorHigh, (vHeight - 0.33) / 0.33);
  } else {
    color = mix(uColorHigh, uColorPeak, (vHeight - 0.66) / 0.34);
  }

  // Hash-derived hue rotation
  color = hueRotate(color, uHueShift);

  // Saturation modulation
  float grey = dot(color, vec3(0.299, 0.587, 0.114));
  color = mix(vec3(grey), color, 0.6 + uSaturation * 0.4);

  // Simple directional shading from screen-space derivatives
  vec3 dx = dFdx(vec3(vUv.x, vHeight * 2.0, vUv.y));
  vec3 dy = dFdy(vec3(vUv.x, vHeight * 2.0, vUv.y));
  vec3 normal = normalize(cross(dx, dy));
  vec3 lightDir = normalize(vec3(-0.4, 0.8, 0.4));
  float diffuse = max(dot(normal, lightDir), 0.0);
  color *= 0.4 + diffuse * 0.6;

  // Exponential fog
  float fogFactor = 1.0 - exp(-uFogDensity * vFogDepth * uFogDensity * vFogDepth);
  color = mix(color, uFogColor, clamp(fogFactor, 0.0, 1.0));

  gl_FragColor = vec4(color, uOpacity);
}
