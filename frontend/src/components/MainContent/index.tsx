'use client';

import React from 'react';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';

interface MainContentProps {
  children: React.ReactNode;
}

const MainContent: React.FC<MainContentProps> = ({ children }) => {
  // The page is inset by the rail whenever the sidebar isn't taking a column — including
  // a narrow window's expanded overlay.
  const { isContentInsetCollapsed } = useSidebar();

  return (
    <main
      className={`min-w-0 flex-1 pb-[var(--rail-h)] transition-all duration-300 ${
        isContentInsetCollapsed ? 'ml-16' : 'ml-64'
      }`}
    >
      {children}
    </main>
  );
};

export default MainContent;
