import { describe, it, expect } from 'vitest';
import { filterPeople, type FilterablePerson } from '@/lib/people-filter';

const people: FilterablePerson[] = [
  { displayName: 'Brian Scates', role: 'Founder', email: 'brian@example.com' },
  { displayName: 'Jordan Lee', role: 'Engineer', email: 'jordan.lee@acme.io' },
  { displayName: 'Priya Patel', role: null, email: null },
];

describe('filterPeople (WS3.3)', () => {
  it('returns everyone for an empty / whitespace query', () => {
    expect(filterPeople(people, '')).toHaveLength(3);
    expect(filterPeople(people, '   ')).toHaveLength(3);
  });

  it('matches on display name, case-insensitively', () => {
    expect(filterPeople(people, 'brian').map((p) => p.displayName)).toEqual(['Brian Scates']);
    expect(filterPeople(people, 'PRIYA').map((p) => p.displayName)).toEqual(['Priya Patel']);
  });

  it('matches on role and on email', () => {
    expect(filterPeople(people, 'engineer').map((p) => p.displayName)).toEqual(['Jordan Lee']);
    expect(filterPeople(people, 'acme.io').map((p) => p.displayName)).toEqual(['Jordan Lee']);
  });

  it('matches a partial substring across fields', () => {
    expect(filterPeople(people, 'e').length).toBeGreaterThan(1);
  });

  it('returns nothing when no field matches', () => {
    expect(filterPeople(people, 'zzz-nobody')).toEqual([]);
  });

  it('tolerates null role/email without throwing', () => {
    expect(() => filterPeople(people, 'priya')).not.toThrow();
    expect(filterPeople(people, 'priya')).toHaveLength(1);
  });
});
