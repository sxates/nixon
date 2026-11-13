'use client';

import { Suspense } from 'react';
import { useSearchParams } from 'next/navigation';
import { TopicView } from '@/components/Topics/TopicView';
import { EmptyState, PageLoading } from '@/components/ui/empty-state';

function TopicPageContent() {
  const id = useSearchParams().get('id');
  if (!id) return <EmptyState title="This topic no longer exists" />;
  return <TopicView key={id} id={id} />;
}

export default function TopicPage() {
  // Static export: the id rides in the query string (same pattern as tasks/item).
  return (
    <Suspense fallback={<PageLoading className="bg-background" />}>
      <TopicPageContent />
    </Suspense>
  );
}
