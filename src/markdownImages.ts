import type { Element, Root, RootContent } from 'hast';
import { embeddedMedia } from './mediaSources';

function imageBlock(node: RootContent) {
  return node.type === 'element' && node.tagName === 'p' && node.children.some(child => child.type === 'element' && child.tagName === 'img')
    && node.children.every(child => child.type === 'text' ? !child.value.trim()
      : child.type === 'element' && child.tagName === 'img' && embeddedMedia(String(child.properties.src ?? ''), String(child.properties.alt ?? ''))?.kind === 'image');
}

// Keep image groups in their original position. Prose, lists and other media
// separate groups; code examples never become image previews.
export function groupMarkdownImages(root: Root) {
  const visit = (parent: Root | Element) => {
    const children: RootContent[] = [];
    let gallery: Element | undefined;
    for (const node of parent.children) {
      if (node.type === 'element' && imageBlock(node)) {
        const images = node.children.filter(child => child.type === 'element');
        if (!gallery) {
          gallery = { ...node, tagName: 'div', properties: { className: ['chat-image-gallery'], role: 'group', 'aria-label': '图片组' }, children: [] };
          children.push(gallery);
        }
        gallery.children.push(...images);
        gallery.properties['data-image-count'] = gallery.children.length;
      } else if (gallery && node.type === 'text' && !node.value.trim()) {
        // Markdown emits newlines between paragraphs; they aren't grid cells.
        continue;
      } else {
        gallery = undefined;
        if (node.type === 'element' && !['pre', 'code'].includes(node.tagName)) visit(node);
        children.push(node);
      }
    }
    parent.children = children as Element['children'];
  };
  visit(root);
}
