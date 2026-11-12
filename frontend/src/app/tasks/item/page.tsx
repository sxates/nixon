'use client';

import { Suspense } from 'react';
import { useSearchParams } from 'next/navigation';
import { ItemDetail } from '@/components/ActionItems/ItemDetail';
import { EmptyState, PageLoading } from '@/components/ui/empty-state';

function TaskItemContent() {
  const id = useSearchParams().get('id');
  if (!id) return <EmptyState title="This to-do no longer exists" />;
  return <ItemDetail key={id} id={id} />;
}

export default function TaskItemPage() {
  // Static export: the id rides in the query string (same pattern as meeting-details).
  return (
    <Suspense fallback={<PageLoading className="bg-background" />}>
      <TaskItemContent />
    </Suspense>
  );
}
