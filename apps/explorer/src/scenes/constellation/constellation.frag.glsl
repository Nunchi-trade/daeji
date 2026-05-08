precision highp float;

varying vec4 vColor;

void main() {
  // Soft circle from GL_POINTS point sprite
  vec2 coord = gl_PointCoord - 0.5;
  float dist = length(coord);

  // Smooth falloff: solid center, soft edge
  float alpha = 1.0 - smoothstep(0.3, 0.5, dist);

  // Discard fully transparent fragments
  if (alpha < 0.01) discard;

  gl_FragColor = vec4(vColor.rgb, vColor.a * alpha);
}
