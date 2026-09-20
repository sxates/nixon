'use client';

import React, { useState, useEffect, Suspense } from 'react';
import { useSearchParams } from 'next/navigation';
import { LoaderIcon } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { TranscriptSettings } from '@/components/TranscriptSettings';
import { RecordingSettings } from '@/components/RecordingSettings';
import { AppearanceSettings } from '@/components/AppearanceSettings';
import { PreferenceSettings } from '@/components/PreferenceSettings';
import { RecordingPermissionsSettings } from '@/components/RecordingPermissionsSettings';
import { CalendarSettings } from '@/components/CalendarSettings';
import { OwnerEmailSettings } from '@/components/OwnerEmailSettings';
import { SummaryModelSettings } from '@/components/SummaryModelSettings';
import { TemplateSettings } from '@/components/TemplateSettings';
import { DeveloperSettings } from '@/components/DeveloperSettings';
import { About } from '@/components/About';
import { useConfig } from '@/contexts/ConfigContext';
import { Tabs, TabsList, TabsTrigger, TabsContent } from '@/components/ui/tabs';
import { PageHeader } from '@/components/ui/page-header';

// Tabs configuration (constant)
const TABS = [
  { value: 'general', label: 'General' },
  { value: 'recording', label: 'Recordings' },
  { value: 'Transcriptionmodels', label: 'Transcription' },
  { value: 'summaryModels', label: 'Summary' },
  { value: 'templates', label: 'Templates' },
  { value: 'about', label: 'About' }
] as const;

/**
 * The Developer tab (specs/0059) only exists in debug builds, and only became a tab of its
 * own in specs/0066 W3: it used to ride along under "Beta", which had exactly one feature —
 * Import Audio & Retranscribe. With import graduated there are no beta features left, so a
 * "Beta" tab in a release build would be an empty room. `dev_get_flags` is a debug-only
 * command, so its presence is the build check.
 */
const DEV_TAB = { value: 'developer', label: 'Developer' } as const;

function SettingsPageContent() {
  const { transcriptModelConfig, setTranscriptModelConfig } = useConfig();

  // specs/0060 — the screenshot pipeline reaches a specific tab via `/settings?tab=<value>`;
  // an unrecognized (or absent) value falls back to the existing 'general' default.
  const searchParams = useSearchParams();
  const requestedTab = searchParams.get('tab');
  const initialTab =
    TABS.some((t) => t.value === requestedTab) || requestedTab === DEV_TAB.value
      ? (requestedTab as string)
      : 'general';


  // Probed once: a release build's `dev_get_flags` does not exist, so the tab never shows.
  const [devToolsAvailable, setDevToolsAvailable] = useState(false);
  useEffect(() => {
    invoke('dev_get_flags')
      .then(() => setDevToolsAvailable(true))
      .catch(() => setDevToolsAvailable(false));
  }, []);

  const [activeTab, setActiveTab] = useState(initialTab);

  // A deep link can name a section as well as a tab (`#calendar`, specs/0066). The tab
  // strip already handles `?tab=`; this scrolls the named section into view once its
  // content has mounted, so Today's "Connect" lands on the calendar choice rather than at
  // the top of General. `requestAnimationFrame` because the TabsContent for the requested
  // tab is not in the DOM on the first paint.
  useEffect(() => {
    if (typeof window === 'undefined') return;
    const id = window.location.hash.slice(1);
    if (!id) return;
    const frame = requestAnimationFrame(() => {
      document.getElementById(id)?.scrollIntoView({ block: 'start', behavior: 'auto' });
    });
    return () => cancelAnimationFrame(frame);
  }, [activeTab]);

  // Load saved transcript configuration on mount
  useEffect(() => {
    const loadTranscriptConfig = async () => {
      try {
        const config = await invoke('api_get_transcript_config') as any;
        if (config) {
          console.log('Loaded saved transcript config:', config);
          setTranscriptModelConfig({
            provider: config.provider || 'localWhisper',
            model: config.model || 'large-v3',
            // Masked hint only — raw keys never cross IPC (spec 0030 WS2)
            apiKey: config.apiKeyMasked ?? null
          });
        }
      } catch (error) {
        console.error('Failed to load transcript config:', error);
      }
    };
    loadTranscriptConfig();
  }, [setTranscriptModelConfig]);

  // The Tabs root is the page container so the TabsList can live in the fixed
  // header while the TabsContent panels scroll below it.
  return (
    <Tabs
      value={activeTab}
      onValueChange={setActiveTab}
      className="flex h-page flex-col bg-background"
    >
      {/* Fixed header — the shared PageHeader. The tab strip sits directly under
          the title so it stays visible while the tab content scrolls. */}
      <PageHeader title="Settings" className="pb-0" />

      <div className="flex-shrink-0 px-7">
        {/* Pure-CSS underline (specs/0057 Task 2): the active tab owns a 2px brand
            border-bottom, the same formula as the meeting-details tab bar. The old
            measured framer-motion bar could land in the wrong place on first paint. */}
        <TabsList className="mt-3 h-auto rounded-none border-b border-border bg-transparent p-0">
          {[...TABS, ...(devToolsAvailable ? [DEV_TAB] : [])].map((tab) => (
            <TabsTrigger
              key={tab.value}
              value={tab.value}
              // Hidden while a screenshot driver is attached (`data-shot="1"`, globals.css):
              // this tab exists only in debug builds, so a README capture must not show it.
              {...(tab.value === DEV_TAB.value ? { 'data-dev-only': '' } : {})}
              className="-mb-px rounded-none border-0 border-b-2 border-transparent bg-transparent px-3 py-1.5 text-sm font-semibold text-muted-foreground transition-colors [transition-duration:140ms] hover:text-foreground data-[state=active]:border-brand data-[state=active]:bg-transparent data-[state=active]:text-foreground data-[state=active]:shadow-none"
            >
              {tab.label}
            </TabsTrigger>
          ))}
        </TabsList>
      </div>

      {/* Scrollable content — only this area scrolls. */}
      <div className="flex-1 overflow-y-auto px-7 pb-12">
        <div className="mx-auto max-w-[1080px] pt-6">
          <TabsContent value="general" className="space-y-8">
            {/* General order: Appearance → Notifications → Recording permissions →
                Calendar → Your email → the rest. */}
            {/* Appearance (specs/0057 decision 1) — Faceplate / Deck / System. */}
            <AppearanceSettings />
            <PreferenceSettings />
            {/* Recording permissions (spec 0038 WS7.a) — mic + audio-capture
                status with an action that opens the same first-run permissions
                modal. Replaces the former top-level "Permissions" sidebar entry. */}
            <RecordingPermissionsSettings />
            {/* Calendar source (0008 EventKit + 0032 Google) — lives under General
                because it's an app-wide source choice, not a recording knob. */}
            <CalendarSettings />
            {/* Owner addresses (specs/0018) — auto-filled with the Google account
                email when Google Calendar connects (specs/0032). */}
            <OwnerEmailSettings />
          </TabsContent>
          <TabsContent value="recording" className="space-y-8">
            <RecordingSettings />
          </TabsContent>
          <TabsContent value="Transcriptionmodels" className="space-y-8">
            <TranscriptSettings
              transcriptModelConfig={transcriptModelConfig}
              setTranscriptModelConfig={setTranscriptModelConfig}
            />
          </TabsContent>
          <TabsContent value="summaryModels" className="space-y-8">
            <SummaryModelSettings />
          </TabsContent>
          <TabsContent value="templates" className="space-y-8">
            <TemplateSettings />
          </TabsContent>
          <TabsContent value="developer" className="space-y-8">
            <DeveloperSettings />
          </TabsContent>
          {/* About (spec 0038 WS7.c) — version, credits, and links. Reuses the
              same <About /> body that was previously a standalone modal. */}
          <TabsContent value="about" className="mt-6">
            <About />
          </TabsContent>
        </div>
      </div>
    </Tabs>
  );
}

export default function SettingsPage() {
  return (
    <Suspense fallback={
      <div className="flex items-center justify-center h-page">
        <LoaderIcon className="animate-spin size-6" />
      </div>
    }>
      <SettingsPageContent />
    </Suspense>
  );
}
