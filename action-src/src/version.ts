declare const __BUILD_VERSION__: string;
declare const __MIN_VERSION__: string;

export const BUILD_VERSION: string =
  typeof __BUILD_VERSION__ === 'string' ? __BUILD_VERSION__ : '0.0.0-dev';
export const MIN_VERSION: string =
  typeof __MIN_VERSION__ === 'string' ? __MIN_VERSION__ : '0.0.0-dev';
