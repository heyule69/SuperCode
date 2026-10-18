import { useEffect, useLayoutEffect, useRef, useState, type RefObject } from 'react';
export function SelectionActions({area,add,details,ask}:{area:RefObject<HTMLElement|null>;add:(text:string)=>void;details:(text:string)=>void;ask:(text:string)=>void}){
  const [selected,setSelected]=useState<{text:string;rect:DOMRect;user:boolean}|null>(null);
  const toolbar=useRef<HTMLDivElement>(null);
  useEffect(()=>{
    const element=area.current;if(!element)return;
    function capture(event:Event){
      if(event instanceof KeyboardEvent && event.key==='Escape')return;
      const sel=window.getSelection();
      if(!sel?.rangeCount||sel.isCollapsed||!element!.contains(sel.anchorNode)||!element!.contains(sel.focusNode)){setSelected(null);return;}
      const text=sel.toString().trim().slice(0,64*1024);
      const message=(node:Node|null)=>(node instanceof Element?node:node?.parentElement)?.closest('.message.user');
      const user=message(sel.anchorNode);
      setSelected(text?{text,rect:sel.getRangeAt(0).getBoundingClientRect(),user:!!user&&user===message(sel.focusNode)}:null);
    }
    element.addEventListener('mouseup',capture);element.addEventListener('keyup',capture);
    const dismiss=()=>setSelected(null);
    const outside=(event:PointerEvent)=>{if(!toolbar.current?.contains(event.target as Node))dismiss();};
    const selectionChanged=()=>{if(window.getSelection()?.isCollapsed)dismiss();};
    element.addEventListener('scroll',dismiss,true);window.addEventListener('resize',dismiss);
    document.addEventListener('pointerdown',outside);document.addEventListener('selectionchange',selectionChanged);
    return()=>{element.removeEventListener('mouseup',capture);element.removeEventListener('keyup',capture);element.removeEventListener('scroll',dismiss,true);window.removeEventListener('resize',dismiss);document.removeEventListener('pointerdown',outside);document.removeEventListener('selectionchange',selectionChanged);};
  },[area]);
  useEffect(()=>{const escape=(e:KeyboardEvent)=>{if(e.key==='Escape')setSelected(null);};window.addEventListener('keydown',escape);return()=>window.removeEventListener('keydown',escape);},[]);
  useLayoutEffect(()=>{
    const node=toolbar.current;if(!node||!selected)return;
    const bounds=node.getBoundingClientRect();
    node.style.left=`${Math.max(12,Math.min(selected.rect.left,window.innerWidth-bounds.width-12))}px`;
    const above=selected.rect.top-bounds.height-8;
    node.style.top=`${Math.max(12,Math.min(above>=12?above:selected.rect.bottom+8,window.innerHeight-bounds.height-12))}px`;
    node.style.visibility='visible';
  },[selected]);
  if(!selected)return null;
  function run(fn:(text:string)=>void){fn(selected!.text);window.getSelection()?.removeAllRanges();setSelected(null);}
  return <div ref={toolbar} className="selection-actions" role="toolbar" aria-label="选中文字操作" style={{left:12,top:12,visibility:'hidden'}} onMouseDown={e=>e.preventDefault()}>{selected.user?null:<><button onClick={()=>run(add)}>添加到对话</button><button onClick={()=>run(details)}>更多详情</button></>}<button onClick={()=>run(ask)}>在侧边聊天中提问</button></div>;
}
