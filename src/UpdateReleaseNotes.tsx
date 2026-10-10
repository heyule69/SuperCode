import ReactMarkdown from 'react-markdown';

export default function UpdateReleaseNotes({ notes, openLink }: { notes: string; openLink: (url: string) => void }) {
  return <ReactMarkdown skipHtml
    allowedElements={['p', 'ul', 'ol', 'li', 'strong', 'em', 'del', 'code', 'a', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'blockquote', 'br']}
    urlTransform={url => { try { return new URL(url).protocol === 'https:' ? url : ''; } catch { return ''; } }}
    components={{
      h1: ({ children }) => <h5>{children}</h5>, h2: ({ children }) => <h5>{children}</h5>, h3: ({ children }) => <h5>{children}</h5>,
      h4: ({ children }) => <h5>{children}</h5>, h5: ({ children }) => <h5>{children}</h5>, h6: ({ children }) => <h5>{children}</h5>,
      a: ({ href, children }) => href ? <a href={href} onClick={e => { e.preventDefault(); openLink(href); }}>{children}</a> : <span>{children}</span>,
    }}>{notes}</ReactMarkdown>;
}
