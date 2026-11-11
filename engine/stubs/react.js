// Minimal React stand-in: the engine only uses JSX for display formatting.
export function createElement(type, props, ...children) {
  return { type, props, children };
}
export const Fragment = 'fragment';
export class Component {}
export default { createElement, Fragment, Component };
