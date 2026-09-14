import NotesRedirect from './NotesRedirect';

interface PageProps {
  params: {
    id: string;
  };
}

/**
 * Static-export needs a param list for the dynamic `[id]` segment. Real meeting ids are
 * runtime values (UUIDs) we can't enumerate at build time; the client component resolves the
 * actual id from the URL. We list the historical placeholder ids so those paths pre-render.
 */
export function generateStaticParams() {
  return [
    { id: 'team-sync-dec-26' },
    { id: 'product-review' },
    { id: 'project-ideas' },
    { id: 'action-items' },
  ];
}

/**
 * The standalone notes view was retired in spec 0003 (pivot 2026-06-24): notes now live in the
 * meeting-details "My Notes" tab. This route redirects there so old links don't break.
 */
const NotePage = ({ params }: PageProps) => {
  return <NotesRedirect paramId={params.id} />;
};

export default NotePage;
