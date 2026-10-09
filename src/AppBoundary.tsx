import { Component, type ReactNode } from 'react';

export class AppBoundary extends Component<{ children: ReactNode }, { failed: boolean }> {
  state = { failed: false };
  static getDerivedStateFromError() { return { failed: true }; }
  render() {
    return this.state.failed ? <main className="app-recovery"><h2>界面暂时无法显示</h2><p>对话保存在本地。重新加载会恢复当前会话，并继续显示正在运行的任务。</p><button onClick={() => window.location.reload()}>重新加载</button></main> : this.props.children;
  }
}
