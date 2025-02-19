gamepad = {
    gamepadIndex: null,
    buttons: [],
    axes: [],
    isConnected: false,
    name: '',
    buttonPressures: {},
    axesPressures: {},
    throttledEvents: {},
    buttonOverwrite: { leftButton: null, rightButton: null, aButton: null, bButton: null },
    playerLimit: 2,
    assignedControllers: {},
    spritesheetButtonMap: {
      ps5: ['up','left','down','right','leftstick','leftstickpressed','rightstick','rightstickpressed','x','square','circle','triangle','l1','l2','r1','r2'],
      xbox: ['up','left','down','right','leftstick','leftstickpressed','rightstick','rightstickpressed','a','x','b','y','lb','lt','rb','rt'],
      switch: ['up','left','down','right','leftstick','leftstickpressed','rightstick','rightstickpressed','b','y','a','x','l','zl','r','zr']
    },
    defaultPlatform: 'ps5',
    button_size: 16,
    display_size: 32,
  
    start() {
        assets.preload([
          { name: 'gamepad_buttons', path: 'plugins/gamepad/buttons.png' }
        ]);
        console.log(`Plugin started: ${this.id}`)
        plugin.loadedPlugins[this.id] = this;
        
        this.listenerIds = {
          connect: input.assign('gamepadconnected', (e) => this.connectGamepad(e)),
          disconnect: input.assign('gamepaddisconnected', (e) => this.disconnectGamepad(e)),
          aButton: input.assign('gamepada', () => plugin.hook('onA')),
          bButton: input.assign('gamepadb', () => plugin.hook('onB')),
          yPressed: input.assign('gamepadyPressed', this.throttle(() => plugin.hook('onY'), 1000)),
          yReleased: input.assign('gamepadyReleased', this.throttle(() => plugin.hook('onYRelease'), 1000)),
          xPressed: input.assign('gamepadxPressed', () => plugin.hook('onX')),
          xReleased: input.assign('gamepadxReleased', () => plugin.hook('onXRelease')),
          yButton: input.assign('gamepady', () => plugin.hook('onY')),
          l1Pressed: input.assign('gamepadl1Pressed', (e) => plugin.hook('onL1', {detail: e.detail})),
          startPressed: input.assign('gamepadstartPressed', this.throttle(() => plugin.hook('onStart'), 1000)),
          axes: input.assign('gamepadAxes', (e) => this.handleAxes(e.detail)),
          l2Pressed: input.assign('gamepadl2Pressed', (e) => plugin.hook('onL2', {detail: e})),
          r2Pressed: input.assign('gamepadr2Pressed', this.throttle((e) => plugin.hook('onR2', {detail: e}), 50)),
          r2Released: input.assign('gamepadr2Released', this.throttle(() => plugin.hook('onR2Release'), 50)),
          l2Released: input.assign('gamepadl2Released', () => plugin.hook('onL2Release')),
          rightStickPressed: input.assign('gamepadrightStickPressed', () => plugin.hook('onrightStickPress'), 50)
        }
        
        this.loop = setInterval(() => this.updateGamepadState(), 16)
    },

    unmount() {
        clearInterval(this.loop)
        Object.values(this.listenerIds).forEach(id => input.destroy(id))
        this.isConnected = false
        this.gamepadIndex = null
        console.log(`Plugin unmounted: ${this.id}`)
    },
  
    onRender() {
      this.updateGamepadState()
    },
  
    throttle(func, delay) {
      return (...args) => {
        const now = Date.now()
        const lastCall = this.throttledEvents[func] || 0
        if (now - lastCall >= delay) {
          this.throttledEvents[func] = now
          func(...args)
        }
      }
    },

connectGamepad(e) {
    this.gamepadIndex = e.gamepad.index
    this.isConnected = true
    this.name = this.getGamepadName(e.gamepad)
    input.updateInputMethod('gamepad', this.name)
    const event = new CustomEvent('gamepadConnected')
    window.dispatchEvent(event)
  },

  disconnectGamepad(e) {
    if (e.gamepad.index === this.gamepadIndex) {
      this.isConnected = false
      this.gamepadIndex = null
      input.updateInputMethod('keyboard')
      const event = new CustomEvent('gamepadDisconnected')
      window.dispatchEvent(event)
    }
  },

  updateGamepadState() {
    if (this.isConnected && this.gamepadIndex !== null) {
      const gp = navigator.getGamepads()[this.gamepadIndex]
      if (gp) {
        if (this.hasActiveInput(gp)) {
          input.updateInputMethod('gamepad', this.name)
        }
        this.handleButtons(gp.buttons)
        const axesEvent = new CustomEvent('gamepadAxes', { detail: gp.axes })
        window.dispatchEvent(axesEvent)
      }
    }
  },

  hasActiveInput(gp) {
    const t = 0.2
    const bp = gp.buttons.some(b => b.pressed)
    const am = gp.axes.some(a => Math.abs(a) > t)
    return bp || am
  },

  handleButtons(buttons) {
    const names = this.getButtonNames();
    buttons.forEach((btn, i) => {
        const pressure = btn.value;
        this.buttonPressures[i] = pressure;
        const n = names[i];
        
        if (!n) return;
        
        const upperN = n.charAt(0).toUpperCase() + n.slice(1);
        
        if (btn.pressed) {
            plugin.hook(`on${n}`, {pressure});
            plugin.hook(`on${upperN}`, {pressure});
            
            if (!this.buttons.includes(n)) {
                this.buttons.push(n);
                plugin.hook(`on${n}Pressed`, {pressure});
                plugin.hook(`on${upperN}Pressed`, {pressure});
            }
        } else {
            const idx = this.buttons.indexOf(n);
            if (idx > -1) {
                this.buttons.splice(idx, 1);
                plugin.hook(`on${n}Released`, {pressure});
                plugin.hook(`on${upperN}Released`, {pressure});
            }
        }
    });
},

getButtonNames() {
    return [
        'a','b','x','y',
        'l1','r1','l2','r2',
        'select','start',
        'leftStick','rightStick',
        'up','down','left','right'
    ];
},

  assignController(player, gamepadIndex) {
    if (Object.keys(this.assignedControllers).length < this.playerLimit) {
      this.assignedControllers[player] = gamepadIndex
    }
  },

  unassignController(player) {
    if (this.assignedControllers[player]) {
      delete this.assignedControllers[player]
    }
  },

  getGamepadName(gp) {
    const map = {
      '045e:02e0': 'Xbox 360',
      '045e:028e': 'Xbox One',
      '054c:0ce6': 'PS5',
      '054c:05c4': 'PS4',
      '057e:2009': 'Nintendo Switch Pro Controller',
      '046d:c216': 'Logitech F310',
      '1038:1412': 'SteelSeries Stratus Duo'
    }
    const id = gp.id
    const m = id.match(/Vendor:\s*([0-9a-fA-F]{4})\s*Product:\s*([0-9a-fA-F]{4})/)
    if (m) {
      const vp = `${m[1].toLowerCase()}:${m[2].toLowerCase()}`
      if (map[vp]) return map[vp]
    }
    if (id.toLowerCase().includes('xbox')) return 'Xbox'
    if (id.toLowerCase().includes('nintendo') || id.toLowerCase().includes('switch')) return 'Switch'
    if (id.toLowerCase().includes('logitech')) return 'Logitech'
    if (id.toLowerCase().includes('steelseries')) return 'SteelSeries'
    return 'Generic'
  },

  vibrate(d, strong=1.0, weak=1.0) {
    const gp = navigator.getGamepads()[this.gamepadIndex]
    if (this.isConnected && gp && gp.vibrationActuator) {
      gp.vibrationActuator.playEffect('dual-rumble', { 
        duration: d, 
        startDelay: 0, 
        strongMagnitude: strong, 
        weakMagnitude: weak 
      }).catch(err => console.log(err))
    }
  },

  handleAxes(axes) {
    this.handleLeftAxes(axes)
    this.handleRightAxes(axes)
  },

  handleLeftAxes(axes) {
    const t = 0.1
    const lx = axes[0]
    const ly = axes[1]
    if (!game.mainSprite) return

    plugin.hook('onLeftStickMove', {x: lx, y: ly})

    const s = game.mainSprite
    if (s.isVehicle) {
      if (Math.abs(lx) > t) s.updateVehicleDirection(lx, game.deltaTime)
      else s.updateVehicleDirection(0, game.deltaTime)
      const r2p = this.axesPressures.rightTrigger || 0
      const l2p = this.axesPressures.leftTrigger || 0
      if (r2p < t && l2p < t) {
        s.currentSpeed = Math.max(0, s.currentSpeed - s.braking * (game.deltaTime/1000))
      }
      s.moveVehicle()
    } else {
      this.directions = { left: false, right: false, up: false, down: false }
      if (Math.abs(lx) > t || Math.abs(ly) > t) {
        const p = Math.min(1, Math.sqrt(lx**2 + ly**2))
        const ms = 10
        const ts = s.topSpeed
        const r2Boost = this.buttons.includes('r2') ? 1.0 : 0.6
        const relSp = ms + (p * (ts-ms) * r2Boost)
        s.speed = Math.min(relSp, ts)
        const angle = Math.atan2(ly, lx)
        this.updateGamepadDirections(angle)
        this.updateSpriteDirections()
      } else {
        this.axesPressures.leftStickX = 0
        this.axesPressures.leftStickY = 0
        if (s) s.speed = 0
        this.updateSpriteDirections()
      }
    }
  },

  handleRightAxes(axes) {
    if (!game.mainSprite) return
    const d = 0.1
    const rx = axes[2]
    const ry = axes[3]

    plugin.hook('onRightStickMove', {x: rx, y: ry})

    if (Math.abs(rx) > d || Math.abs(ry) > d) {
      this.axesPressures.rightStickX = Math.abs(rx)
      this.axesPressures.rightStickY = Math.abs(ry)
      if (game.mainSprite && game.mainSprite.targetAim) {
        const as = 10
        const nx = game.mainSprite.targetX + rx * as
        const ny = game.mainSprite.targetY + ry * as
        const dx = nx - (game.mainSprite.x + game.mainSprite.width/2)
        const dy = ny - (game.mainSprite.y + game.mainSprite.height/2)
        const dist = Math.sqrt(dx*dx + dy*dy)
        const ang = Math.atan2(dy, dx)
        
        this.updateSpriteDirection(ang)
        
        if (dist <= game.mainSprite.maxRange) {
          game.mainSprite.targetX = nx
          game.mainSprite.targetY = ny
        } else {
          const mx = game.mainSprite.x + game.mainSprite.width/2 + Math.cos(ang) * game.mainSprite.maxRange
          const my = game.mainSprite.y + game.mainSprite.height/2 + Math.sin(ang) * game.mainSprite.maxRange
          game.mainSprite.targetX = Math.max(0, Math.min(mx, game.worldWidth))
          game.mainSprite.targetY = Math.max(0, Math.min(my, game.worldHeight))
        }
      }
    } else {
      this.axesPressures.rightStickX = 0
      this.axesPressures.rightStickY = 0
    }
  },

  updateGamepadDirections(a) {
    const up = (a >= -Math.PI/8 && a < Math.PI/8)
    const upRight = (a >= Math.PI/8 && a < 3*Math.PI/8)
    const right = (a >= 3*Math.PI/8 && a < 5*Math.PI/8)
    const downRight = (a >= 5*Math.PI/8 && a < 7*Math.PI/8)
    const down = (a >= 7*Math.PI/8 || a < -7*Math.PI/8)
    const downLeft = (a >= -7*Math.PI/8 && a < -5*Math.PI/8)
    const left = (a >= -5*Math.PI/8 && a < -3*Math.PI/8)
    const upLeft = (a >= -3*Math.PI/8 && a < -Math.PI/8)

    if (up) { this.directions.right = true }
    else if (upRight) { this.directions.down = true; this.directions.right = true }
    else if (right) { this.directions.down = true }
    else if (downRight) { this.directions.down = true; this.directions.left = true }
    else if (down) { this.directions.left = true }
    else if (downLeft) { this.directions.up = true; this.directions.left = true }
    else if (left) { this.directions.up = true }
    else if (upLeft) { this.directions.up = true; this.directions.right = true }
  },

  updateSpriteDirections() {
    if (!game.allowControls) return
    const combo = {
      up: (this.directions && this.directions.up) || false,
      down: (this.directions && this.directions.down) || false,
      left: (this.directions && this.directions.left) || false,
      right: (this.directions && this.directions.right) || false
    }
    const arr = ['up','down','left','right']
    arr.forEach(d => {
      if (game.mainSprite) {
        if (combo[d]) game.mainSprite.addDirection(d)
        else game.mainSprite.removeDirection(d)
      }
    })
    if (game.mainSprite && !combo.up && !combo.down && !combo.left && !combo.right) {
      plugin.audio.stopLoopingAudio('footsteps1','sfx',0.5)
    }
  },

  updateSpriteDirection(ang) {
    if (!game.mainSprite) return
    if (ang >= -Math.PI/8 && ang < Math.PI/8) game.mainSprite.direction = 'E'
    else if (ang >= Math.PI/8 && ang < 3*Math.PI/8) game.mainSprite.direction = 'SE'
    else if (ang >= 3*Math.PI/8 && ang < 5*Math.PI/8) game.mainSprite.direction = 'S'
    else if (ang >= 5*Math.PI/8 && ang < 7*Math.PI/8) game.mainSprite.direction = 'SW'
    else if (ang >= 7*Math.PI/8 || ang < -7*Math.PI/8) game.mainSprite.direction = 'W'
    else if (ang >= -7*Math.PI/8 && ang < -5*Math.PI/8) game.mainSprite.direction = 'NW'
    else if (ang >= -5*Math.PI/8 && ang < -3*Math.PI/8) game.mainSprite.direction = 'N'
    else if (ang >= -3*Math.PI/8 && ang < -Math.PI/8) game.mainSprite.direction = 'NE'
  },

  updateButtonImages() {
    const sheet = assets.use('gamepad_buttons')
    let p = this.name || this.defaultPlatform
    if (!this.spritesheetButtonMap[p]) p = this.defaultPlatform
    const arr = this.spritesheetButtonMap[p]
    const row = this.getPlatformRow(p)
    const sc = this.display_size / this.button_size
    if (!arr) return
    arr.forEach((b, i) => {
      const els = document.querySelectorAll(`.gamepad_button_${b}`)
      const x = i*this.button_size
      const y = row*this.button_size
      els.forEach(el => {
        el.style.width = `${this.display_size}px`
        el.style.height = `${this.display_size}px`
        el.style.backgroundImage = `url('${sheet.src}')`
        el.style.backgroundPosition = `-${x*sc}px -${y*sc}px`
        el.style.backgroundSize = `${this.button_size*16*sc}px ${this.button_size*3*sc}px`
        el.style.backgroundRepeat = 'no-repeat'
        el.style.display = 'inline-block'
      })
    })
  },

  clearButtonImages() {
    const arr = this.spritesheetButtonMap.ps5
    arr.forEach(b => {
      const els = document.querySelectorAll(`.gamepad_button_${b}`)
      els.forEach(el => { el.style.backgroundImage = '' })
    })
  },

  getPlatformRow(p) {
    const map = { ps5: 0, xbox: 1, switch: 2 }
    return map[p] !== undefined ? map[p] : 0
  }
}