export const PROVIDERS: Record<string, string> = {
  homebrew: "Homebrew", nvm: "nvm", pyenv: "pyenv", sdkman: "SDKMAN!", asdf: "asdf", fnm: "fnm", manual: "手动安装", system: "系统自带",
  "nvm-windows": "nvm-windows", "pyenv-win": "pyenv-win", mise: "mise", scoop: "Scoop", "scoop-global": "Scoop（全局）", winget: "winget",
  chocolatey: "Chocolatey", installer: "安装程序", rustup: "rustup", rbenv: "rbenv", rvm: "RVM", chruby: "chruby",
};
export const providerName = (p: string) => PROVIDERS[p] || p;
export const viaLabel = (v: string) => (v === "runa" ? "runa 下载" : providerName(v));

export const ORIGINS: Record<string, string> = {
  brew: "Homebrew", winget: "winget", scoop: "Scoop", script: "官方脚本", git: "git", system: "系统", unknown: "未知", powershell: "PowerShell",
};
export const originName = (o: string) => ORIGINS[o] || o;
