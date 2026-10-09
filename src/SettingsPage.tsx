import { useState, type ReactNode } from 'react';
import { Bell, ChartNoAxesColumn, Cpu, Gauge, Globe, Info, Palette, Puzzle, Search, Settings2, Sparkles, SquareTerminal, UserRound, Zap } from 'lucide-react';
import type { ClientTab } from './ClientSettings';
import type { Preferences } from './preferences';
import type { Agent } from './types';
import { compatiblePermission } from './permissions';
import { PermissionMenu } from './ComposerMenus';
import { AgentMenu } from './AgentMenu';

export type SettingsTab = 'general' | 'providers' | 'agents' | 'resources' | 'usage' | 'quota' | 'about' | ClientTab;
const groups = [
  { label: '个人', items: [
    { id: 'general', label: '常规', icon: Settings2, terms: '权限 默认 Agent 发送 快捷键' },
    { id: 'notifications', label: '通知', icon: Bell, terms: '系统 任务完成 失败 审批 提示音 后台 隐私' },
    { id: 'appearance', label: '外观', icon: Palette, terms: '主题 字体 字号 动画' },
    { id: 'personalization', label: '个性化', icon: UserRound, terms: '自定义指令' },
  ] },
  { label: '工作区', items: [
    { id: 'providers', label: '模型供应商', icon: Zap, terms: '连接 API Key 密钥 导入 智普 Kimi' },
    { id: 'agents', label: 'Agent', icon: SquareTerminal, terms: 'Codex Claude CLI 路径' },
    { id: 'skills', label: '技能', icon: Sparkles, terms: 'recall save slash 命令' },
  ] },
  { label: '集成', items: [
    { id: 'plugins', label: '插件与 MCP', icon: Puzzle, terms: '工具 服务' },
    { id: 'automation', label: '自动化工具', icon: Globe, terms: '浏览器 电脑 Playwright' },
  ] },
  { label: '应用', items: [
    { id: 'usage', label: '用量', icon: ChartNoAxesColumn, terms: 'Token 统计' },
    { id: 'quota', label: '额度', icon: Gauge, terms: '套餐 余额 重置 平台 连接 Coding Plan' },
    { id: 'resources', label: '资源', icon: Cpu, terms: '内存 进程 释放' },
    { id: 'about', label: '关于', icon: Info, terms: '版本 SuperCode 更新 GitHub' },
  ] },
] as const;

export function SettingsPage({ tab, select, blocked, children }: { tab: SettingsTab; select: (tab: SettingsTab) => void; blocked: boolean; children: ReactNode }) {
  const [search, setSearch] = useState('');
  const query = search.trim().toLowerCase();
  const shown = groups.map(group => ({ ...group, items: group.items.filter(item => `${item.label} ${item.terms}`.toLowerCase().includes(query)) })).filter(group => group.items.length);
  return <main className="settings-modal settings-page" inert={blocked} aria-label="设置">
    <aside className="settings-navigation">
      <header className="settings-page-header"><h1>设置</h1></header>
      <label className="settings-search"><Search size={16} /><input aria-label="搜索设置" placeholder="搜索" value={search} onChange={e => setSearch(e.target.value)} /></label>
      <nav className="settings-nav" aria-label="设置分类">{shown.map(group => <section className="settings-nav-group" key={group.label} aria-label={group.label}><h2>{group.label}</h2>{group.items.map(item => <button key={item.id} className={tab === item.id ? 'selected' : ''} aria-current={tab === item.id ? 'page' : undefined} onClick={() => select(item.id)}><item.icon size={17} />{item.label}</button>)}</section>)}{!shown.length ? <p className="settings-no-results" role="status">没有匹配的设置</p> : null}</nav>
    </aside>
    <div className={`settings-body settings-${tab}`} key={tab}><div className="settings-content">{children}</div></div>
  </main>;
}

export function SettingsGroup({ title, children }: { title: string; children: ReactNode }) {
  return <section className="settings-group" aria-label={title}><h3>{title}</h3><div className="settings-card">{children}</div></section>;
}
export function SettingRow({ title, description, children }: { title: string; description?: string; children: ReactNode }) {
  return <div className="setting-row"><div className="setting-description"><strong>{title}</strong>{description ? <p>{description}</p> : null}</div><div className="setting-control">{children}</div></div>;
}
export function SettingsSwitch({ label, checked, change, disabled }: { label: string; checked: boolean; change: (checked: boolean) => void; disabled?: boolean }) {
  return <input type="checkbox" role="switch" className="settings-switch" aria-label={label} checked={checked} disabled={disabled} onChange={e => change(e.target.checked)} />;
}
export function GeneralSettings({ prefs, setPrefs, busy, autoExpand, setAutoExpand, agents }: { prefs: Preferences; setPrefs: (prefs: Preferences) => void; busy: boolean; autoExpand: boolean; setAutoExpand: (expand: boolean) => void; agents: Agent[] }) {
  return <><div className="settings-page-heading"><h2>常规</h2></div>
    <SettingsGroup title="权限"><SettingRow title="默认权限" description="用于下一次新聊天，也可以在输入框中单独调整。"><PermissionMenu label="默认权限模式" agent={prefs.defaultAgent} value={compatiblePermission(prefs.defaultPermission, prefs.defaultAgent)} busy={busy} onChange={mode => setPrefs({ ...prefs, defaultPermission: mode })} /></SettingRow></SettingsGroup>
    <SettingsGroup title="聊天"><SettingRow title="默认 Agent" description="新聊天使用的本机 Agent。"><AgentMenu label="默认 Agent" value={prefs.defaultAgent} agents={agents} busy={busy} choose={value => setPrefs({ ...prefs, defaultAgent: value, defaultPermission: compatiblePermission(prefs.defaultPermission, value) })}/></SettingRow>
      <SettingRow title="Enter 发送" description="开启后，Shift Enter 换行；关闭后，Ctrl Enter 发送。"><SettingsSwitch label="Enter 发送" checked={prefs.enterSend} change={value => setPrefs({ ...prefs, enterSend: value })} /></SettingRow>
      <SettingRow title="默认展开执行详情" description="展开思考摘要与工具记录。"><SettingsSwitch label="默认展开思考摘要与工具详情" checked={autoExpand} change={setAutoExpand} /></SettingRow>
    </SettingsGroup>
    <SettingsGroup title="键盘快捷键">{[['新建聊天', 'Ctrl N'], ['搜索聊天', 'Ctrl K'], ['设置', 'Ctrl ,'], ['切换侧栏', 'Ctrl B']].map(([title, key]) => <SettingRow key={title} title={title}><kbd>{key}</kbd></SettingRow>)}</SettingsGroup>
  </>;
}
