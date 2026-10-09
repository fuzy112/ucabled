import { useCallback, useState } from "react";
import Boot from "../sections/Boot";
import Header from "../sections/Header";
import Hero from "../sections/Hero";
import Marquee from "../sections/Marquee";
import Shots from "../sections/Shots";
import HowItWorks from "../sections/HowItWorks";
import DumbPipe from "../sections/DumbPipe";
import Install from "../sections/Install";
import Ssh from "../sections/Ssh";
import Compat from "../sections/Compat";
import Limits from "../sections/Limits";
import Footer from "../sections/Footer";
import Particles from "../components/Particles";

export default function Home() {
  const [booted, setBooted] = useState(false);
  const done = useCallback(() => setBooted(true), []);

  return (
    <div className="crt relative min-h-screen bg-black">
      {!booted && <Boot onDone={done} />}
      <Particles />
      <div className="relative z-10">
        <Header />
        <main>
          <Hero />
          <Marquee />
          <HowItWorks />
          <Shots />
          <DumbPipe />
          <Install />
          <Ssh />
          <Compat />
          <Limits />
        </main>
        <Footer />
      </div>
    </div>
  );
}
