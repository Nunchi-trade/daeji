/// <reference types="vite/client" />

declare module '*.glsl' {
  const value: string;
  export default value;
}
declare module '*.vert.glsl' {
  const value: string;
  export default value;
}
declare module '*.frag.glsl' {
  const value: string;
  export default value;
}
declare module '*.module.css' {
  const classes: { readonly [key: string]: string };
  export default classes;
}
