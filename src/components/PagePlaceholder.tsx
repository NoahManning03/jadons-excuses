import { motion } from "framer-motion";

interface Props {
  eyebrow: string;
  title: string;
  subtitle: string;
}

export function PagePlaceholder({ eyebrow, title, subtitle }: Props) {
  return (
    <div className="min-h-full px-10 py-16">
      <div className="mx-auto max-w-3xl">
        <motion.div
          initial={{ opacity: 0, y: 6 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.4, ease: "easeOut" }}
          className="rounded-2xl border border-slate-100 bg-white p-10 shadow-soft"
        >
          <p className="text-xs font-medium uppercase tracking-[0.18em] text-tangerine-600">
            {eyebrow}
          </p>
          <h2
            className="mt-3 text-3xl tracking-tightish text-slate-900"
            style={{ fontWeight: 650 }}
          >
            {title}
          </h2>
          <p className="mt-3 max-w-xl text-base text-slate-600">{subtitle}</p>
        </motion.div>
      </div>
    </div>
  );
}
