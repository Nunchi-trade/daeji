precision highp float;

uniform sampler2D uHeightmap;
uniform float uActivityLevel;
uniform float uTime;

varying float vHeight;
varying float vFogDepth;
varying vec2 vUv;

void main() {
  vUv = uv;

  // Sample heightmap texture at this vertex's UV
  float h = texture2D(uHeightmap, uv).r;
  vHeight = h;

  // Displace Y by height.
  // Activity level scales amplitude:
  //   empty blocks (activity=0) -> max displacement 0.6
  //   full blocks  (activity=1) -> max displacement 2.0
  float maxHeight = 0.6 + uActivityLevel * 1.4;
  vec3 pos = position;
  pos.y += h * maxHeight;

  // Subtle wave animation on peak vertices
  pos.y += sin(uTime * 0.5 + pos.x * 0.3 + pos.z * 0.2) * 0.02 * h;

  vec4 mvPos = modelViewMatrix * vec4(pos, 1.0);
  vFogDepth = -mvPos.z;

  gl_Position = projectionMatrix * mvPos;
}
