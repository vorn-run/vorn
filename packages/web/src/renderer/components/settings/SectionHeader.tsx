export function SectionHeader({ label }: { label: string }) {
  return (
    <div className="text-[10px] text-gray-600 uppercase tracking-wider font-medium px-3 pt-5 pb-1.5">
      {label}
    </div>
  )
}
