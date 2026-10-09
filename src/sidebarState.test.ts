import { describe, expect, it } from 'vitest';
import { emptySidebar, exportFilename, projectSessions, sidebarAreas, sidebarGroups } from './sidebarState';
import type { Project, Session, SidebarState } from './types';
const projects: Project[] = [{ id:'p1',name:'项目一',path:'D:/one' }, { id:'p2',name:'项目二',path:'D:/two' }];
const session = (id:string,projectId='p1'):Session => ({ id, projectId,title:id,agent:'claude',model:null,nativeId:null,status:'idle',updatedAt:0,turnId:null });
const sessions=[session('s1'),session('s2'),session('s3','p2'),session('missing','gone')];
describe('sidebar grouping',()=>{
  it('places pins first and does not duplicate pinned projects or chats',()=>{
    const state:SidebarState={projects:{p2:{pinned:true,unread:false,sectionId:null}},sessions:{s1:{pinned:true,unread:false,sectionId:null}},sections:[]};
    const groups=sidebarGroups(projects,sessions,state);
    expect(groups[0].id).toBe('pinned');expect(groups[0].projects.map(p=>p.id)).toEqual(['p2']);expect(groups[0].sessions.map(s=>s.id)).toEqual(['s1']);
    expect(groups[1].projects.map(p=>p.id)).toEqual(['p1']);expect(projectSessions('p1',sessions,state).map(s=>s.id)).toEqual(['s2']);
  });
  it('supports project sections and independent chat sections without losing the project association',()=>{
    const state:SidebarState={projects:{p1:{pinned:false,unread:false,sectionId:'work'}},sessions:{s2:{pinned:false,unread:true,sectionId:'work'}},sections:[{id:'work',name:'工作'}]};
    const groups=sidebarGroups(projects,sessions,state);const work=groups.find(g=>g.id==='work')!;
    expect(work.projects.map(p=>p.id)).toEqual(['p1']);expect(work.sessions.map(s=>s.id)).toEqual(['s2']);
    expect(projectSessions('p1',sessions,state).map(s=>s.id)).toEqual(['s1']);expect(work.sessions[0].projectId).toBe('p1');
  });
  it('returns chats to their project when a section is missing or deleted',()=>{
    const state={...emptySidebar,sessions:{s1:{pinned:false,unread:false,sectionId:'deleted'}}};
    expect(projectSessions('p1',sessions,state).map(s=>s.id)).toEqual(['s1','s2']);
    expect(sidebarGroups(projects,sessions,state).flatMap(g=>g.sessions)).toEqual([]);
  });
  it('does not render records without a visible project and keeps empty custom sections manageable',()=>{
    const state:SidebarState={...emptySidebar,sections:[{id:'empty',name:'空分区'}],sessions:{missing:{pinned:true,unread:false,sectionId:null}}};
    const groups=sidebarGroups(projects,sessions,state);expect(groups.flatMap(g=>g.sessions)).toEqual([]);expect(groups.at(-1)?.id).toBe('empty');
  });
  it('creates safe filenames while preserving Chinese titles',()=>{
    expect(exportFilename('规划 SuperCode / 测试?', 'html')).toBe('规划 SuperCode _ 测试_.html');
    expect(exportFilename('...','md')).toBe('聊天.md');expect(exportFilename('x'.repeat(200),'md').length).toBe(83);
  });
});


it('shows projectless chats in recents, pins and custom sections without hiding or duplicating them', () => {
  const standalone = [session('recent', ''), session('pin', ''), session('work', '')];
  const state: SidebarState = { projects: {}, sessions: { pin: { pinned:true, unread:false, sectionId:null }, work: { pinned:false, unread:false, sectionId:'custom' } }, sections:[{id:'custom',name:'工作'}] };
  const groups = sidebarGroups([], standalone, state);
  expect(groups.find(g=>g.id==='recent')?.sessions.map(s=>s.id)).toEqual(['recent']);
  expect(groups.find(g=>g.id==='pinned')?.sessions.map(s=>s.id)).toEqual(['pin']);
  expect(groups.find(g=>g.id==='custom')?.sessions.map(s=>s.id)).toEqual(['work']);
  expect(groups.flatMap(g=>g.sessions).length).toBe(3);
});

it('separates direct chats from projects while keeping pins and custom sections in their own area', () => {
  const rows = [session('project-pin'), session('project-section'), session('project-normal'), session('direct-pin', ''), session('direct-section', ''), session('direct-recent', '')];
  const state: SidebarState = {
    projects: { p1:{pinned:true,unread:false,sectionId:null} },
    sessions: {
      'project-pin':{pinned:true,unread:false,sectionId:null},
      'project-section':{pinned:false,unread:false,sectionId:'work'},
      'direct-pin':{pinned:true,unread:true,sectionId:null},
      'direct-section':{pinned:false,unread:false,sectionId:'work'},
    },
    sections:[{id:'work',name:'工作'},{id:'empty',name:'空分区'}],
  };
  const areas = sidebarAreas(projects, rows, state);
  expect(areas.projects.flatMap(group=>group.sessions).map(row=>row.id)).toEqual(['project-pin','project-section']);
  expect(areas.projects.flatMap(group=>group.projects).map(row=>row.id)).toEqual(['p1','p2']);
  expect(projectSessions('p1', rows, state).map(row=>row.id)).toEqual(['project-normal']);
  expect(areas.direct.flatMap(group=>group.sessions).map(row=>row.id)).toEqual(['direct-pin','direct-recent','direct-section']);
  expect(areas.direct.every(group=>!group.projects.length)).toBe(true);
  expect(areas.projects.some(group=>group.id==='empty')).toBe(true);
  expect(areas.direct.some(group=>group.id==='empty')).toBe(false);
});

it('keeps direct chats accessible with no projects and hides chats belonging to removed projects', () => {
  const areas = sidebarAreas([], [session('direct', ''), session('missing-project')], emptySidebar);
  expect(areas.projects).toEqual([]);
  expect(areas.direct.flatMap(group=>group.sessions).map(row=>row.id)).toEqual(['direct']);
});
