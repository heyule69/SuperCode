import { useEffect, useState, type RefObject } from 'react';
export function SelectionActions({area,add,details,ask}:{area:RefObject<HTMLElement|null>;add:(text:string)=>void;details:(text:string)=>void;ask:(text:string)=>void}){
  const [selected,setSelected]=useState<{text:string;x:number;y:number}|null>(null);
  useEffect(()=>{
    const element=area.current;if(!element)return;
    function capture(){const sel=window.getSelection();if(!sel?.rangeCount||sel.isCollapsed||!element!.contains(sel.anchorNode)||!element!.contains(sel.focusNode)){setSelected(null);return;}const text=sel.toString().trim().slice(0,64*1024);const rect=sel.getRangeAt(0).getBoundingClientRect();setSelected(text?{text,x:Math.max(12,Math.min(rect.left,window.innerWidth-330)),y:Math.max(42,rect.top-40)}:null);}
    element.addEventListener('mouseup',capture);element.addEventListener('keyup',capture);
    const dismiss=()=>setSelected(null);element.addEventListener('scroll',dismiss);window.addEventListener('resize',dismiss);
    return()=>{element.removeEventListener('mouseup',capture);element.removeEventListener('keyup',capture);element.removeEventListener('scroll',dismiss);window.removeEventListener('resize',dismiss);};
  },[area]);
  useEffect(()=>{const escape=(e:KeyboardEvent)=>{if(e.key==='Escape')setSelected(null);};window.addEventListener('keydown',escape);return()=>window.removeEventListener('keydown',escape);},[]);
  if(!selected)return null;
  function run(fn:(text:string)=>void){fn(selected!.text);window.getSelection()?.removeAllRanges();setSelected(null);}
  return <div className="selection-actions" role="toolbar" aria-label="选中文字操作" style={{left:selected.x,top:selected.y}} onMouseDown={e=>e.preventDefault()}><button onClick={()=>run(add)}>添加到对话</button><button onClick={()=>run(details)}>更多详情</button><button onClick={()=>run(ask)}>在侧边聊天中提问</button></div>;
}
