import Section from "../components/Section";
import Reveal from "../components/Reveal";
import Rich from "../components/Rich";
import CopyButton from "../components/CopyButton";
import { useCopy } from "../lib/i18n";

const SSH = `# phones have no non-resident mode: every key is a discoverable
# passkey on the phone; the key handle ssh needs lives in the private key file
ssh-keygen -t ecdsa-sk -f ~/.ssh/id_ecdsa_sk

# -O resident is accepted but changes little on a phone, since
# every passkey is already discoverable there
ssh-keygen -t ecdsa-sk -O resident -f ~/.ssh/id_ecdsa_resident

# deploy + log in
ssh-copy-id -i ~/.ssh/id_ecdsa_sk.pub you@example.com
ssh -i ~/.ssh/id_ecdsa_sk you@example.com

# sign / verify a file, no ssh server involved
ssh-keygen -Y sign -f ~/.ssh/id_ecdsa_sk -n file ./file
ssh-keygen -Y verify -f allowed_signers -I "$USER" -n file -s ./file.sig < ./file`;

export default function Ssh() {
  const c = useCopy();
  return (
    <Section num="05" id="ssh" title={c.ssh.title}>
      <Reveal>
        <p className="mb-6 max-w-3xl text-[13px] leading-relaxed" style={{ color: "var(--phos-55)" }}>
          <Rich text={c.ssh.intro} />
        </p>
      </Reveal>
      <Reveal delay={80}>
        <div className="notch p-5 sm:p-6">
          <div className="mb-4 flex items-center justify-between">
            <span className="text-[10px] uppercase tracking-widest" style={{ color: "var(--phos-40)" }}>
              {c.ssh.label}
            </span>
            <CopyButton text={SSH} />
          </div>
          <pre className="codeblock">{SSH}</pre>
        </div>
      </Reveal>
      <Reveal delay={140}>
        <p className="mt-5 text-[12px]" style={{ color: "var(--phos-40)" }}>
          {c.ssh.note}
        </p>
      </Reveal>
    </Section>
  );
}
